//! This module provides the low-level primitives that are used to build
//! the state implementation for V1 smart contracts.
//!
//! Functions in this module are, as the name of the module suggests, low-level
//! and generally have many preconditions, violation of which will make them
//! unsafe, could trigger panics, or memory corruption. For this reason
//! functions should only be used via the exposed high-level api in the
//! `super::api` module, which is re-exported at the top-level.
use super::types::*;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use sha2::Digest;
use slab::Slab;
use std::{
    collections::HashMap,
    io::{Read, Write},
    iter::once,
    num::NonZeroU16,
    slice::Iter,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

const INLINE_CAPACITY: usize = 8;

#[derive(Default, Debug)]
/// An inner node in the [PrefixMap]. The default instance produces an empty
/// node with no values and no children.
struct InnerNode {
    value:    Option<NonZeroU16>,
    /// Children ordered by increasing keys.
    children: Vec<KeyIndexPair>,
}

#[derive(Debug)]
/// A prefix map that efficiently stores a list of keys and supports the
/// following operations
/// - insert with reference counting
/// - delete
/// - check whether the given key is extended by any value in the collection
/// - check whether the given key either extends any value or is extended by any
///   value
///
/// The data structure is a basic trie. Instead of using pointers to children
/// node we use a slab of nodes, and children are pointers in this vector. This
/// is to avoid issues with lifetimes and ownership when traversing and
/// modifying the tree.
pub(crate) struct PrefixesMap {
    /// Root of the map. This is [None] if and only if the map is empty.
    /// If this is Some then the index is the key in the [PrefixesMap::nodes]
    /// slab below.
    root:  Option<usize>,
    /// All the live nodes in the tree.
    nodes: Slab<InnerNode>,
}

impl PrefixesMap {
    pub fn new() -> Self {
        PrefixesMap {
            root:  None,
            nodes: Slab::new(),
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool { self.root.is_none() }

    pub fn insert(&mut self, key: &[u8]) -> Result<(), TooManyIterators> {
        let mut node_idx = if let Some(root) = self.root {
            root
        } else {
            let root = self.nodes.insert(InnerNode::default());
            self.root = Some(root);
            root
        };
        for k in key {
            let node = unsafe { self.nodes.get_unchecked_mut(node_idx) };
            match node.children.binary_search_by_key(k, |x| x.key()) {
                Ok(idx) => {
                    let c = unsafe { node.children.get_unchecked(idx) };
                    node_idx = c.index();
                }
                Err(idx) => {
                    let new_node = self.nodes.insert(InnerNode::default());
                    // look up again to not have issues with double mutable borrow.
                    // This could be improved.
                    let node = unsafe { self.nodes.get_unchecked_mut(node_idx) };
                    node.children.insert(idx, KeyIndexPair::new(*k, new_node));
                    node_idx = new_node;
                }
            }
        }
        let node = unsafe { self.nodes.get_unchecked_mut(node_idx) };
        if let Some(value) = node.value {
            let new_value = value.get().checked_add(1).ok_or(TooManyIterators)?;
            node.value = Some(unsafe { NonZeroU16::new_unchecked(new_value) });
        } else {
            node.value = Some(unsafe { NonZeroU16::new_unchecked(1) });
        }
        Ok(())
    }

    /// Return whether the given key has a prefix in the prefix map.
    pub fn check_has_no_prefix(&self, key: &[u8]) -> Result<(), AttemptToModifyLockedArea> {
        let mut node_idx = if let Some(root) = self.root {
            root
        } else {
            // empty tree
            return Ok(());
        };
        for k in key {
            let node = unsafe { self.nodes.get_unchecked(node_idx) };
            // if there is a value at this node, then we have found our prefix.
            if node.value.is_some() {
                return Err(AttemptToModifyLockedArea);
            }
            if let Ok(idx) = node.children.binary_search_by_key(k, |x| x.key()) {
                let c = unsafe { node.children.get_unchecked(idx) };
                node_idx = c.index();
            } else {
                return Ok(());
            }
        }
        // we found a node that either has a value, or has children, in the first case
        // the given key is a prefix of some value in the trie. In the latter it is not,
        // the entire tree is below it.
        let node = unsafe { self.nodes.get_unchecked(node_idx) };
        if node.value.is_some() {
            Err(AttemptToModifyLockedArea)
        } else {
            Ok(())
        }
    }

    /// Return whether any key in the trie **is a prefix of the given key**, or
    /// whether the given key **is extended by** any keys in the map.
    pub fn is_or_has_prefix(&self, key: &[u8]) -> bool {
        let mut node_idx = if let Some(root) = self.root {
            root
        } else {
            // empty tree
            return false;
        };
        for k in key {
            let node = unsafe { self.nodes.get_unchecked(node_idx) };
            // if there is a value at this node, then we have found our prefix.
            if node.value.is_some() {
                return true;
            }
            if let Ok(idx) = node.children.binary_search_by_key(k, |x| x.key()) {
                let c = unsafe { node.children.get_unchecked(idx) };
                node_idx = c.index();
            } else {
                return false;
            }
        }
        // we found a node that either has a value, or has children. In the first case
        // it matches the key exactly, so is a prefix of it. In the latter, a key
        // extends this node.
        true
    }

    /// Delete the given key from the map. That is, decrease the reference count
    /// by 1, and if this is the last occurrence of the key remove it from the
    /// map. Return whether the key was in the map.
    pub fn delete(&mut self, key: &[u8]) -> bool {
        let mut node_idx = if let Some(root) = self.root {
            root
        } else {
            // empty tree
            return false;
        };
        let mut stack = Vec::new();
        for k in key {
            let node = unsafe { self.nodes.get_unchecked(node_idx) };
            if let Ok(idx) = node.children.binary_search_by_key(k, |x| x.key()) {
                let c = unsafe { node.children.get_unchecked(idx) };
                stack.push((node_idx, idx));
                node_idx = c.index();
            } else {
                return false;
            }
        }
        let node = unsafe { self.nodes.get_unchecked_mut(node_idx) };
        let have_removed = node.value.is_some();
        match node.value {
            Some(ref mut value) if value.get() > 1 => {
                *value = unsafe { NonZeroU16::new_unchecked(value.get() - 1) };
                return true;
            }
            _ => node.value = None,
        }
        // back up and delete subtrees if needed
        if node.children.is_empty() {
            self.nodes.remove(node_idx);
            while let Some((node_idx, child_idx)) = stack.pop() {
                let node = unsafe { self.nodes.get_unchecked_mut(node_idx) };
                node.children.remove(child_idx);
                if !node.children.is_empty() || node.value.is_some() {
                    break;
                } else {
                    self.nodes.remove(node_idx);
                }
            }
            // delete the root, if needed
            if let Some(root) = self.root {
                if !self.nodes.contains(root) {
                    self.root = None;
                }
            }
        }
        have_removed
    }
}

#[derive(Debug)]
/// A link to a shared occurrence of a value V.
/// This is used in this module to construct trees, allowing for sharing of
/// values in trees and subtrees in case of the persistent tree.
pub struct Link<V> {
    link: Arc<RwLock<V>>,
}

impl<V> Clone for Link<V> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            link: self.link.clone(),
        }
    }
}

impl<V> Link<V> {
    pub fn new(value: V) -> Self {
        Self {
            link: Arc::new(RwLock::new(value)),
        }
    }

    #[inline(always)]
    /// Immutably borrow the pointed to value.
    pub fn borrow(&self) -> RwLockReadGuard<'_, V> { self.link.as_ref().read().unwrap() }

    #[inline(always)]
    /// Mutably borrow the value that is pointed to.
    pub fn borrow_mut(&self) -> RwLockWriteGuard<'_, V> { self.link.as_ref().write().unwrap() }

    #[inline(always)]
    /// Attempt to consume the link. If the pointed to value has a single owner
    /// this will return Ok(_), otherwise it will return an error.
    pub fn try_unwrap(self) -> Result<V, Self> {
        Arc::try_unwrap(self.link)
            .map_err(|link| Link {
                link,
            })
            .map(|rc| rc.into_inner().expect("Thread panicked."))
    }
}

#[derive(Debug, Clone)]
/// A potentially cached value V. This is a value that can either be purely in
/// memory, purely in backing storage, or both in memory and in backing storage.
pub enum CachedRef<V> {
    Disk {
        key: Reference,
    },
    Memory {
        value: V,
    },
    Cached {
        key:   Reference,
        value: V,
    },
}

/// The default hash implementation is not a valid value.
impl<V> Default for CachedRef<V> {
    fn default() -> Self {
        CachedRef::Disk {
            key: Default::default(),
        }
    }
}

impl<V: Loadable> CachedRef<V> {
    #[inline(always)]
    pub fn get(&self, loader: &mut impl BackingStoreLoad) -> V
    where
        V: Clone, {
        match self {
            CachedRef::Disk {
                key,
                ..
            } => V::load_from_location(loader, *key).unwrap(),
            CachedRef::Memory {
                value,
                ..
            } => value.clone(),
            CachedRef::Cached {
                value,
                ..
            } => value.clone(),
        }
    }

    /// Apply the supplied function to the contained value. The value is loaded
    /// if it is not yet cached. Note that this will **not** cache the
    /// value, the loaded value will be dropped.
    pub fn use_value<X>(&self, loader: &mut impl BackingStoreLoad, f: impl FnOnce(&V) -> X) -> X {
        match self {
            CachedRef::Disk {
                key,
            } => {
                let loaded = V::load_from_location(loader, *key).unwrap();
                f(&loaded)
            }
            CachedRef::Memory {
                value,
                ..
            } => f(value),
            CachedRef::Cached {
                value,
                ..
            } => f(value),
        }
    }
}

impl<V> CachedRef<V> {
    pub fn new(value: V) -> CachedRef<V> {
        CachedRef::Memory {
            value,
        }
    }

    pub fn load_and_cache<F: BackingStoreLoad>(&mut self, loader: &mut F) -> &mut V
    where
        V: Loadable, {
        match self {
            CachedRef::Disk {
                key,
            } => {
                let value = V::load_from_location(loader, *key).unwrap(); // TODO: Error handling.
                *self = CachedRef::Cached {
                    key: *key,
                    value,
                };
                if let CachedRef::Cached {
                    value,
                    ..
                } = self
                {
                    value
                } else {
                    unsafe { std::hint::unreachable_unchecked() }
                }
            }
            CachedRef::Memory {
                value,
            } => value,
            CachedRef::Cached {
                value,
                ..
            } => value,
        }
    }

    /// If the value is in memory, set it to cached with the given key.
    /// Otherwise do nothing. This of course has the precondition that the key
    /// stores the value in the relevant backing store. Internal use only.
    fn cache_with(&mut self, key: Reference) {
        if let CachedRef::Memory {
            ..
        } = self
        {
            let value = std::mem::replace(self, CachedRef::Disk {
                key,
            });
            if let CachedRef::Memory {
                value,
            } = value
            {
                *self = CachedRef::Cached {
                    value,
                    key,
                };
            } else {
                // this is unreachable since we hold a mutable reference to the cached value
                // and we know the value was a purely in-memory one
                unsafe { std::hint::unreachable_unchecked() }
            }
        }
    }

    pub fn store_and_cache<S: BackingStoreStore, W: std::io::Write>(
        &mut self,
        backing_store: &mut S,
        buf: &mut W,
    ) -> StoreResult<()>
    where
        V: AsRef<[u8]>, {
        match self {
            CachedRef::Disk {
                key,
            } => key.store(buf),
            CachedRef::Memory {
                value,
            } => {
                let key = backing_store.store_raw(value.as_ref())?;
                let value = std::mem::replace(self, CachedRef::Disk {
                    key,
                });
                if let CachedRef::Memory {
                    value,
                } = value
                {
                    *self = CachedRef::Cached {
                        value,
                        key,
                    };
                } else {
                    // this is unreachable since we hold a mutable reference to the cached value
                    // and we know the value was a purely in-memory one
                    unsafe { std::hint::unreachable_unchecked() }
                }
                key.store(buf)
            }
            CachedRef::Cached {
                key,
                ..
            } => key.store(buf),
        }
    }

    /// Get a mutable reference to the value, **if it is only in memory**.
    /// Otherwise return the key.
    #[inline]
    pub fn get_mut_or_key(&mut self) -> Result<&mut V, Reference> {
        match self {
            CachedRef::Disk {
                key,
            } => Err(*key),
            CachedRef::Memory {
                value,
            } => Ok(value),
            CachedRef::Cached {
                key,
                ..
            } => Err(*key),
        }
    }

    /// Get a mutable reference to the value, **if it is memory or cached**.
    /// If it is only on disk return None
    #[inline]
    pub fn get_value(self) -> Option<V> {
        match self {
            CachedRef::Disk {
                ..
            } => None,
            CachedRef::Memory {
                value,
            } => Some(value),
            CachedRef::Cached {
                value,
                ..
            } => Some(value),
        }
    }
}

#[derive(Debug, Clone)]
enum Stem {
    // the first byte is length, remaining is data.
    Short([u8; 16]),
    Long(Arc<[KeyPart]>),
}

impl Stem {
    pub fn empty() -> Self { Self::Short([0u8; 16]) }

    fn extend(&mut self, mid: u8, second: &[u8]) {
        match self {
            Stem::Short(buf) => {
                let cur_len = usize::from(buf[0]);
                if cur_len + 1 + second.len() <= 15 {
                    buf[0] += 1 + second.len() as u8;
                    buf[cur_len + 1] = mid;
                    buf[cur_len + 2..cur_len + 2 + second.len()].copy_from_slice(second);
                } else {
                    let new = buf[1..cur_len + 1]
                        .iter()
                        .copied()
                        .chain(once(mid))
                        .chain(second.iter().copied())
                        .collect();
                    *self = Stem::Long(new);
                }
            }
            Stem::Long(arc) => {
                let new = arc
                    .as_ref()
                    .iter()
                    .copied()
                    .chain(once(mid))
                    .chain(second.iter().copied())
                    .collect();
                *self = Stem::Long(new);
            }
        }
    }
}

impl From<&[u8]> for Stem {
    #[inline(always)]
    fn from(s: &[u8]) -> Self {
        let len = s.len();
        if len <= 15 {
            let mut buf = [0u8; 16];
            buf[0] = len as u8;
            buf[1..1 + len].copy_from_slice(s);
            Self::Short(buf)
        } else {
            Self::Long(Arc::from(s))
        }
    }
}

impl From<Vec<u8>> for Stem {
    #[inline(always)]
    fn from(s: Vec<u8>) -> Self {
        let len = s.len();
        if len <= 15 {
            let mut buf = [0u8; 16];
            buf[0] = len as u8;
            buf[1..1 + len].copy_from_slice(&s);
            Self::Short(buf)
        } else {
            Self::Long(Arc::from(s))
        }
    }
}

impl AsRef<[u8]> for Stem {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        match self {
            Stem::Short(s) => {
                let len = usize::from(s[0]);
                &s[1..1 + len]
            }
            Stem::Long(arc) => arc.as_ref(),
        }
    }
}

/// Recursive link to a child node.
type ChildLink<V> = Link<CachedRef<Hashed<Node<V>>>>;

#[derive(Debug)]
/// A persistent node. Cloning this is cheap, it only copies pointers and
/// increments reference counts.
pub struct Node<V> {
    /// Since a single node owns each value using Hashed<Cached<V>>
    /// here makes sense, it makes it so that the hash is stored inline.
    value:    Option<Link<Hashed<CachedRef<V>>>>,
    path:     Stem,
    // TODO: It might be better to have just an array or Box here.
    // The Rc has an advantage when thawing since we only copy the Rc
    // but is otherwise annoying.
    // In contrast to Hashed<Cached<..>> above for the value, here we store the hash
    // behind a pointer indirection. The reason for this is that there are going to be many
    // pointers to the same node, and we want to avoid duplicating node hashes.
    children: Vec<(KeyPart, ChildLink<V>)>,
}

impl<V> Drop for Node<V> {
    fn drop(&mut self) {
        let mut stack = Vec::new();
        // if we are the only owner of the children we can deallocate them.
        let children = std::mem::take(&mut self.children);
        for (_, child) in children.into_iter() {
            if let Ok(only_child) = child.try_unwrap() {
                if let Some(memory_child) = only_child.get_value() {
                    stack.push(memory_child.data);
                }
            }
        }
        while let Some(mut node) = stack.pop() {
            let children = std::mem::take(&mut node.children);
            for (_, child) in children.into_iter() {
                if let Ok(only_child) = child.try_unwrap() {
                    if let Some(memory_child) = only_child.get_value() {
                        stack.push(memory_child.data);
                    }
                }
            }
        }
    }
}

impl<V> Clone for Node<V> {
    fn clone(&self) -> Self {
        Self {
            value:    self.value.clone(),
            path:     self.path.clone(),
            children: self.children.clone(),
        }
    }
}

impl<V: Loadable, Ctx: BackingStoreLoad> ToSHA256<Ctx> for CachedRef<Hashed<V>>
where
    V: ToSHA256<Ctx>,
{
    #[inline(always)]
    fn hash(&self, ctx: &mut Ctx) -> Hash { self.use_value(ctx, |v| v.hash(&mut ())) }
}

// TODO: Review and revise for security and correctness.
impl<V, Ctx: BackingStoreLoad> ToSHA256<Ctx> for Node<V> {
    fn hash(&self, ctx: &mut Ctx) -> Hash {
        let mut hasher = sha2::Sha256::new();
        match &self.value {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(value.borrow().hash(ctx));
            }
            None => hasher.update(&[0]),
        }
        hasher.update(&self.path);
        let mut child_hasher = sha2::Sha256::new();
        child_hasher.update(&(self.children.len() as u16).to_be_bytes());
        for child in self.children.iter() {
            child_hasher.update(&[child.0]);
            child_hasher.update(child.1.borrow().hash(ctx));
        }
        hasher.update(child_hasher.finalize());
        let hash: [u8; 32] = hasher.finalize().into();
        Hash::from(hash)
    }
}

#[derive(Debug)]
struct MutableNode<V> {
    generation: u32,
    /// Pointer to the table of entries, if the node has a value.
    value:      Option<EntryId>,
    path:       Stem,
    children:   ChildrenCow<V>,
}

impl<V> ChildrenCow<V> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            ChildrenCow::Borrowed(b) => b.len(),
            ChildrenCow::Owned {
                value,
                ..
            } => value.len(),
        }
    }
}

impl<V> Default for MutableNode<V> {
    fn default() -> Self {
        Self {
            generation: 0,
            value:      None,
            path:       Stem::empty(),
            children:   ChildrenCow::Owned {
                generation: 0,
                value:      tinyvec::TinyVec::new(),
            },
        }
    }
}

impl<V> MutableNode<V> {
    pub fn migrate(&self, entries: &mut Vec<Entry>, generation: u32) -> Self {
        let value = if let Some(idx) = self.value {
            let new_entry_idx = entries.len();
            let entry = entries[idx];
            let new_entry = if let Entry::Mutable {
                entry_idx,
            } = entry
            {
                Entry::ReadOnly {
                    entry_idx,
                    borrowed: false,
                }
            } else {
                entry
            };
            entries.push(new_entry);
            Some(new_entry_idx.into())
        } else {
            None
        };
        Self {
            generation,
            value,
            path: self.path.clone(), // this is a cheap clone as well.
            children: self.children.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// A checkpoint that is saved for a mutable trie so that we can cleanup on
/// state rollback. It stores which items were alive at the time of the
/// checkpoint, utilizing the fact that items are always just added to the end
/// of the relevant collections.
struct Checkpoint {
    pub num_nodes:          usize,
    pub num_values:         usize,
    pub num_borrowed_nodes: usize,
    pub num_entries:        usize,
}

#[derive(Debug)]
/// A generation of the [MutableTrie]. This keeps track of the current root of
/// the tree, together with the enough data to be able to go back to the
/// previous generation. Generations are used to checkpoint the tree. This
/// structure only makes sense in the context of a [MutableTrie], since it
/// maintains pointers into other parts of the trie.
struct Generation {
    /// Pointer to the root node of the trie at this generation. This is [None]
    /// if and only if the trie at this generation is empty.
    root:           Option<usize>,
    /// Checkpoint that allows us to clean up the trie when going back to the
    /// **previous** generation.
    checkpoint:     Checkpoint,
    /// Map of prefixes that are locked by iterators.
    iterator_roots: PrefixesMap,
}

impl Generation {
    /// Construct a generation that contains the given root, no locks, and the
    /// checkpoint which goes back to the beginning of the trie.
    fn new(root: Option<usize>) -> Self {
        Generation {
            root,
            checkpoint: Checkpoint {
                num_nodes:          0,
                num_values:         0,
                num_borrowed_nodes: 0,
                num_entries:        0,
            },
            iterator_roots: PrefixesMap::new(),
        }
    }

    /// Construct a generation that contains the given root and checkpoint, and
    /// no locks.
    fn new_with_checkpoint(root: Option<usize>, checkpoint: Checkpoint) -> Self {
        Generation {
            root,
            checkpoint,
            iterator_roots: PrefixesMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct MutableTrie<V> {
    /// Roots for previous generations.
    generations:     Vec<Generation>,
    /// Entries. These are pointers to either [MutableTrie::values] or
    /// [MutableTrie::borrowed_values].
    entries:         Vec<Entry>,
    values:          Vec<V>,
    borrowed_values: Vec<Link<Hashed<CachedRef<V>>>>,
    /// List of all the nodes for all generations. Nodes for new generations are
    /// always added at the end.
    nodes:           Vec<MutableNode<V>>,
}

#[derive(Debug)]
enum ChildrenCow<V> {
    Borrowed(Vec<(KeyPart, ChildLink<V>)>),
    Owned {
        generation: u32,
        value:      tinyvec::TinyVec<[KeyIndexPair; INLINE_CAPACITY]>,
    },
}

impl<V> ChildrenCow<V> {
    /// Return a reference to the owned value, if the enum is an owned variant.
    /// Otherwise return [None].
    #[inline]
    pub fn get_owned(&self) -> Option<(u32, &[KeyIndexPair])> {
        if let ChildrenCow::Owned {
            generation,
            value,
        } = self
        {
            Some((*generation, value))
        } else {
            None
        }
    }

    /// Return a mutable reference to the owned value, if the enum is an owned
    /// variant. Otherwise return [None].
    #[inline]
    pub fn get_owned_mut(&mut self) -> Option<(u32, &mut [KeyIndexPair])> {
        if let ChildrenCow::Owned {
            generation,
            value,
        } = self
        {
            Some((*generation, value))
        } else {
            None
        }
    }
}

fn freeze_value<Ctx, V: Default + ToSHA256<Ctx>, C: Collector<V>>(
    borrowed_values: &mut [Link<Hashed<CachedRef<V>>>],
    owned_values: &mut [V],
    entries: &[Entry],
    mutable: Option<EntryId>,
    loader: &mut Ctx,
    collector: &mut C,
) -> Option<Link<Hashed<CachedRef<V>>>> {
    let entry_idx = mutable?;
    match entries[entry_idx] {
        Entry::ReadOnly {
            borrowed,
            entry_idx,
            ..
        } => {
            if borrowed {
                Some(borrowed_values[entry_idx].clone())
            } else {
                let value = std::mem::take(&mut owned_values[entry_idx]);
                let hash = value.hash(loader);
                collector.add_value(&value);
                Some(Link::new(Hashed::new(hash, CachedRef::Memory {
                    value,
                })))
            }
        }
        Entry::Mutable {
            entry_idx,
            ..
        } => {
            let value = std::mem::take(&mut owned_values[entry_idx]);
            collector.add_value(&value);
            let hash = value.hash(loader);
            Some(Link::new(Hashed::new(hash, CachedRef::Memory {
                value,
            })))
        }
        Entry::Deleted => None,
    }
}

#[repr(transparent)]
#[derive(Default, Clone, Copy)]
struct KeyIndexPair {
    pub pair: usize,
}

/// Format the [KeyIndexPair] as a pair of a key and index.
impl std::fmt::Debug for KeyIndexPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self.key(), self.index()).fmt(f)
    }
}

impl KeyIndexPair {
    #[inline(always)]
    pub fn key(self) -> KeyPart { (self.pair >> 56) as u8 }

    #[inline(always)]
    pub fn index(self) -> usize { self.pair & 0x00ff_ffff_ffff_ffff }

    #[inline(always)]
    pub fn new(key: KeyPart, index: usize) -> Self {
        let pair = usize::from(key) << 56 | index;
        Self {
            pair,
        }
    }
}

impl<V> Clone for ChildrenCow<V> {
    fn clone(&self) -> Self {
        match self {
            ChildrenCow::Borrowed(rc) => ChildrenCow::Borrowed(rc.clone()),
            ChildrenCow::Owned {
                generation,
                value,
            } => ChildrenCow::Owned {
                generation: *generation,
                value:      value.clone(),
            },
        }
    }
}

impl<V> Loadable for CachedRef<V> {
    #[inline(always)]
    fn load<S: std::io::Read, F: BackingStoreLoad>(
        loader: &mut F,
        source: &mut S,
    ) -> LoadResult<Self> {
        let reference = Reference::load(loader, source)?;
        Ok(CachedRef::Disk {
            key: reference,
        })
    }
}

impl<V> Loadable for Node<V> {
    fn load<S: std::io::Read, F: BackingStoreLoad>(
        loader: &mut F,
        source: &mut S,
    ) -> LoadResult<Self> {
        let (path, has_value) = read_node_path_and_value_tag(source)?;
        let value = if has_value {
            let val = Hashed::<CachedRef<V>>::load(loader, source)?;
            Some(Link::new(val))
        } else {
            None
        };
        let num_branches = source.read_u16::<BigEndian>()?;
        let mut branches = Vec::with_capacity(num_branches.into());
        for _ in 0..num_branches {
            let key = source.read_u8()?;
            let reference = CachedRef::<Hashed<Node<V>>>::load(loader, source)?;
            branches.push((key, Link::new(reference)));
        }
        Ok(Node {
            value,
            path,
            children: branches,
        })
    }
}

impl<V: Loadable> Node<V> {
    /// The entire tree in memory.
    pub fn cache<F: BackingStoreLoad>(&mut self, loader: &mut F) {
        if let Some(v) = self.value.as_mut() {
            v.borrow_mut().data.load_and_cache(loader);
        }
        let mut stack = Vec::new();
        for c in self.children.iter() {
            stack.push(c.1.clone());
        }
        while let Some(node) = stack.pop() {
            let mut node_borrow = node.borrow_mut();
            let node = node_borrow.load_and_cache(loader);
            if let Some(v) = node.data.value.as_mut() {
                v.borrow_mut().data.load_and_cache(loader);
            }
            for c in node.data.children.iter() {
                stack.push(c.1.clone());
            }
        }
    }
}

impl<V: AsRef<[u8]>> Hashed<Node<V>> {
    pub fn store_update<S: BackingStoreStore>(
        &mut self,
        backing_store: &mut S,
    ) -> Result<Vec<u8>, WriteError> {
        let mut buf = Vec::new();
        self.store_update_buf(backing_store, &mut buf)?;
        Ok(buf)
    }

    pub fn store_update_buf<S: BackingStoreStore, W: std::io::Write>(
        &mut self,
        backing_store: &mut S,
        buf: &mut W,
    ) -> StoreResult<()> {
        buf.write_all(&self.hash.as_ref())?;
        self.data.store_update_buf(backing_store, buf)
    }
}

impl<V: AsRef<[u8]>> Node<V> {
    pub fn store_update_buf<S: BackingStoreStore, W: std::io::Write>(
        &mut self,
        backing_store: &mut S,
        buf: &mut W,
    ) -> StoreResult<()> {
        let mut stack = Vec::new();
        for (_, ch) in self.children.iter().rev() {
            stack.push((ch.clone(), false));
        }
        let store_node = |node: &mut Node<V>,
                          buf: &mut Vec<u8>,
                          backing_store: &mut S,
                          ref_stack: &mut Vec<Reference>|
         -> StoreResult<()> {
            write_node_path_and_value_tag(node.path.as_ref(), node.value.is_none(), buf)?;
            // store the path
            buf.write_all(node.path.as_ref())?;
            // store the value
            if let Some(v) = &mut node.value {
                let mut borrowed = v.borrow_mut();
                buf.write_all(borrowed.hash.as_ref())?;
                borrowed.data.store_and_cache(backing_store, buf)?;
            }
            // TODO: Revise this when branching on 4 bits.
            // now store the children.
            buf.write_u16::<BigEndian>(node.children.len() as u16)?;
            for (k, _) in node.children.iter() {
                buf.write_u8(*k)?;
                ref_stack.pop().unwrap().store(buf)?;
            }
            Ok(())
        };
        let mut ref_stack = Vec::<Reference>::new();
        let mut tmp_buf = Vec::new();
        while let Some((node_ref, children_processed)) = stack.pop() {
            let node_ref_clone = node_ref.clone();
            let mut node_ref_mut = node_ref.borrow_mut();
            match node_ref_mut.get_mut_or_key() {
                Ok(hashed_node) => {
                    if children_processed {
                        tmp_buf.clear();
                        tmp_buf.write_all(hashed_node.hash.as_ref())?;
                        store_node(
                            &mut hashed_node.data,
                            &mut tmp_buf,
                            backing_store,
                            &mut ref_stack,
                        )?;
                        let key = backing_store.store_raw(&tmp_buf)?;
                        ref_stack.push(key);
                        node_ref_mut.cache_with(key);
                    } else {
                        stack.push((node_ref_clone, true));
                        for (_, ch) in hashed_node.data.children.iter().rev() {
                            stack.push((ch.clone(), false));
                        }
                    }
                }
                Err(key) => {
                    ref_stack.push(key);
                }
            }
        }
        tmp_buf.clear();
        store_node(self, &mut tmp_buf, backing_store, &mut ref_stack)?;
        buf.write_all(&tmp_buf)?;
        Ok(())
    }
}

/// Make the children owned, and return whether the node has a value, the new
/// length of owned_nodes, and a mutable reference to the children.
fn make_owned<'a, 'b, V>(
    idx: usize,
    borrowed_values: &mut Vec<Link<Hashed<CachedRef<V>>>>,
    owned_nodes: &'a mut Vec<MutableNode<V>>,
    entries: &'a mut Vec<Entry>,
    loader: &'b mut impl BackingStoreLoad,
) -> (bool, usize, &'a mut tinyvec::TinyVec<[KeyIndexPair; INLINE_CAPACITY]>) {
    let owned_nodes_len = owned_nodes.len();
    let node = unsafe { owned_nodes.get_unchecked(idx) };
    let node_generation = node.generation;
    let has_value = node.value.is_some();
    let res = {
        match &node.children {
            ChildrenCow::Borrowed(children) => {
                let mut new_nodes = Vec::with_capacity(children.len());
                let c = children
                    .clone()
                    .iter()
                    .zip(owned_nodes_len..)
                    .map(|((k, node), idx)| {
                        new_nodes.push(node.borrow().thaw(
                            borrowed_values,
                            entries,
                            node_generation,
                            loader,
                        ));
                        KeyIndexPair::new(*k, idx)
                    })
                    .collect();
                Some((new_nodes, c))
            }
            ChildrenCow::Owned {
                generation,
                value,
            } => {
                if *generation == node_generation {
                    None
                } else {
                    let mut new_nodes = Vec::with_capacity(value.len());
                    let c = value
                        .iter()
                        .zip(owned_nodes_len..)
                        .map(|(pair, idx)| {
                            new_nodes
                                .push(owned_nodes[pair.index()].migrate(entries, node_generation));
                            KeyIndexPair::new(pair.key(), idx)
                        })
                        .collect();
                    Some((new_nodes, c))
                }
            }
        }
    };
    if let Some((mut to_add, children)) = res {
        owned_nodes.append(&mut to_add);
        let node = unsafe { owned_nodes.get_unchecked_mut(idx) };
        node.children = ChildrenCow::Owned {
            generation: node_generation,
            value:      children,
        };
    }
    let owned_nodes_len = owned_nodes.len();
    match &mut unsafe { owned_nodes.get_unchecked_mut(idx) }.children {
        ChildrenCow::Borrowed(_) => unsafe { std::hint::unreachable_unchecked() },
        ChildrenCow::Owned {
            value: ref mut children,
            ..
        } => (has_value, owned_nodes_len, children),
    }
}

#[derive(Debug, Clone, Copy)]
enum Entry {
    /// An entry is only borrowed as a read-only entry.
    ReadOnly {
        /// Link to the actual entry. If in the borrowed array then this is
        /// true.
        borrowed:  bool,
        entry_idx: usize,
    },
    /// An entry has been made mutable for the relevant generation.
    Mutable {
        /// Index in the array of values. Borrowed entries are never mutable,
        /// so this is an index in the array of normal entries.
        entry_idx: usize,
    },
    /// The entry is deleted.
    Deleted,
}

impl Entry {
    /// Return whether the entry is still alive, i.e., not [Entry::Deleted].
    pub fn is_alive(&self) -> bool { !matches!(self, Self::Deleted) }

    /// Return whether the entry is owned, i.e., mutable. If so, return the
    /// value it points to.
    pub fn is_owned(self) -> Option<usize> {
        if let Self::Mutable {
            entry_idx,
        } = self
        {
            Some(entry_idx)
        } else {
            None
        }
    }
}

type Position = u16;

#[derive(Debug)]
pub struct Iterator {
    /// The root of the iterator. This is stored to allow removal of the
    /// iterator.
    root:         Box<[u8]>,
    /// Pointer to the table of nodes where the iterator is currently anchored.
    current_node: usize,
    /// Key at the current position of the iterator.
    key:          Vec<u8>,
    /// Next child to look at. This is None if
    /// we have to give out the value at the current node, and Some(_)
    /// otherwise.
    next_child:   Option<Position>,
    /// Stack of parents and next positions, and key lengths of parents
    stack:        Vec<(usize, Position, usize)>,
}

impl Iterator {
    /// Get key the iterator is currently pointing at.
    #[inline(always)]
    pub fn get_key(&self) -> &[u8] { &self.key }

    /// Get the key of which the iterator was initialized with.
    #[inline(always)]
    pub fn get_root(&self) -> &[u8] { &self.root }
}

impl<V> CachedRef<Hashed<Node<V>>> {
    fn thaw(
        &self,
        borrowed_values: &mut Vec<Link<Hashed<CachedRef<V>>>>,
        entries: &mut Vec<Entry>,
        generation: u32,
        loader: &mut impl BackingStoreLoad,
    ) -> MutableNode<V> {
        match self {
            CachedRef::Disk {
                key,
                ..
            } => {
                let node: Node<V> =
                    Node::<V>::load_from_location(loader, *key).expect("Failed to read.");
                node.thaw(borrowed_values, entries, generation)
            }
            CachedRef::Memory {
                value,
                ..
            } => value.data.thaw(borrowed_values, entries, generation),
            CachedRef::Cached {
                value,
                ..
            } => value.data.thaw(borrowed_values, entries, generation),
        }
    }
}

impl<V> Node<V> {
    fn thaw(
        &self,
        borrowed_values: &mut Vec<Link<Hashed<CachedRef<V>>>>,
        entries: &mut Vec<Entry>,
        generation: u32,
    ) -> MutableNode<V> {
        let entry = self.value.as_ref().map(|v| {
            let entry_idx = borrowed_values.len();
            borrowed_values.push(v.clone());
            Entry::ReadOnly {
                borrowed: true,
                entry_idx,
            }
        });
        let entry_idx = entry.map(|e| {
            let len = entries.len();
            entries.push(e);
            len.into()
        });
        MutableNode {
            generation,
            value: entry_idx,
            path: self.path.clone(),
            children: ChildrenCow::Borrowed(self.children.clone()),
        }
    }

    pub fn make_mutable(&self, generation: u32) -> MutableTrie<V> {
        let mut borrowed_values = Vec::new();
        let mut entries = Vec::new();
        let root_node = self.thaw(&mut borrowed_values, &mut entries, generation);
        MutableTrie {
            generations: vec![Generation::new(Some(0))],
            values: Vec::new(),
            nodes: vec![root_node],
            borrowed_values,
            entries,
        }
    }
}

impl<V> MutableTrie<V> {
    pub fn empty() -> Self {
        Self {
            generations:     vec![Generation::new(None)],
            values:          Vec::new(),
            nodes:           Vec::new(),
            borrowed_values: Vec::new(),
            entries:         Vec::new(),
        }
    }

    /// Check whether the current generation is an empty tree.
    pub fn is_empty(&self) -> bool { self.generations.last().map_or(false, |x| x.root.is_none()) }
}

impl<V> MutableTrie<V> {
    pub fn new_generation(&mut self) {
        let num_nodes = self.nodes.len();
        let num_values = self.values.len();
        let num_borrowed_nodes = self.borrowed_values.len();
        let num_entries = self.entries.len();
        if let Some(generation) = self.generations.last() {
            let checkpoint = Checkpoint {
                num_nodes,
                num_values,
                num_borrowed_nodes,
                num_entries,
            };
            if let Some(root_idx) = generation.root {
                let root = &self.nodes[root_idx];
                let current_generation = root.generation;
                let new_root_node = root.migrate(&mut self.entries, current_generation + 1);
                let new_root_idx = self.nodes.len();
                self.nodes.push(new_root_node);
                let new_generation =
                    Generation::new_with_checkpoint(Some(new_root_idx), checkpoint);
                self.generations.push(new_generation);
            } else {
                let new_generation = Generation::new_with_checkpoint(None, checkpoint);
                self.generations.push(new_generation);
            }
        }
    }

    /// Pop a generation, removing all data that is only accessible from newer
    /// generations. Return None if no generations are left.
    pub fn pop_generation(&mut self) -> Option<()> {
        let generation = self.generations.pop()?;
        let checkpoint = generation.checkpoint;
        self.nodes.truncate(checkpoint.num_nodes);
        self.values.truncate(checkpoint.num_values);
        self.borrowed_values.truncate(checkpoint.num_borrowed_nodes);
        self.entries.truncate(checkpoint.num_entries);
        Some(())
    }

    /// Modify the tree so that the given root is the latest trie generation.
    /// If that root is already the latest, or does not even exist, this does
    /// nothing.
    pub fn normalize(&mut self, root: u32) {
        let new_len = root as usize + 1;
        let generation = self.generations.get(new_len);
        if let Some(generation) = generation {
            self.nodes.truncate(generation.checkpoint.num_nodes);
            self.values.truncate(generation.checkpoint.num_values);
            self.borrowed_values.truncate(generation.checkpoint.num_borrowed_nodes);
            self.entries.truncate(generation.checkpoint.num_entries);
        }
        self.generations.truncate(new_len);
    }

    /// Get a mutable reference to an entry, if the entry exists. This copies
    /// the data pointed to by the entry unless the entry was already
    /// mutable.
    pub fn get_mut(
        &mut self,
        entry: EntryId,
        loader: &mut impl BackingStoreLoad,
    ) -> Option<&mut V>
    where
        V: Clone + Loadable, {
        let values = &mut self.values;
        let borrowed_entries = &mut self.borrowed_values;
        let entries = &mut self.entries;
        match entries[entry] {
            Entry::ReadOnly {
                borrowed,
                entry_idx,
            } => {
                let value_idx = values.len();
                if borrowed {
                    values.push(borrowed_entries[entry_idx].borrow().data.get(loader));
                } else {
                    values.push(values[entry_idx].clone());
                }
                self.entries[entry] = Entry::Mutable {
                    entry_idx: value_idx,
                };
                values.last_mut()
            }
            Entry::Mutable {
                entry_idx,
            } => values.get_mut(entry_idx),
            Entry::Deleted => None,
        }
    }

    pub fn next<L: BackingStoreLoad, C: TraversalCounter>(
        &mut self,
        loader: &mut L,
        iterator: &mut Iterator,
        counter: &mut C,
    ) -> Result<Option<EntryId>, C::Err> {
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let entries = &mut self.entries;
        loop {
            let node_idx = iterator.current_node;
            let node = &owned_nodes[node_idx];
            let next_child = if let Some(next_child) = iterator.next_child {
                next_child
            } else {
                iterator.next_child = Some(0);
                if node.value.is_some() {
                    return Ok(node.value);
                }
                0
            };
            if usize::from(next_child) < node.children.len() {
                // we have to visit this child.
                iterator.stack.push((node_idx, next_child + 1, iterator.key.len()));
                iterator.next_child = None;
                let (_, _, children) =
                    make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                let child = children[usize::from(next_child)];
                iterator.current_node = child.index();
                let new_path = owned_nodes[iterator.current_node].path.as_ref();
                counter.tick(1 + new_path.len() as u64)?;
                iterator.key.push(child.key());
                iterator.key.extend_from_slice(new_path);
            } else {
                // pop back up.
                if let Some((parent_idx, next_child, key_len)) = iterator.stack.pop() {
                    counter.tick(iterator.key.len().saturating_sub(key_len) as u64)?;
                    iterator.key.truncate(key_len);
                    iterator.current_node = parent_idx;
                    iterator.next_child = Some(next_child);
                } else {
                    // we are done
                    return Ok(None);
                }
            }
        }
    }

    /// Deletes an iterator.
    /// If an iterator was deleted then return `true` otherwise `false`.
    pub fn delete_iter(&mut self, iterator: &Iterator) -> bool {
        let generations = &mut self.generations;
        if let Some(generation) = generations.last_mut() {
            generation.iterator_roots.delete(iterator.get_root())
        } else {
            false
        }
    }

    pub fn iter(
        &mut self,
        loader: &mut impl BackingStoreLoad,
        key: &[KeyPart],
    ) -> Result<Option<Iterator>, TooManyIterators> {
        let mut key_iter = key.iter();
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let entries = &mut self.entries;
        let generation = if let Some(generation) = self.generations.last_mut() {
            generation
        } else {
            return Ok(None);
        };
        let mut node_idx = if let Some(node_idx) = generation.root {
            node_idx
        } else {
            return Ok(None);
        };
        loop {
            let node = unsafe { owned_nodes.get_unchecked_mut(node_idx) };
            let mut stem_iter = node.path.as_ref().iter();
            match follow_stem(&mut key_iter, &mut stem_iter) {
                FollowStem::Equal => {
                    generation.iterator_roots.insert(key)?;
                    return Ok(Some(Iterator {
                        root:         key.into(),
                        current_node: node_idx,
                        key:          key.into(),
                        next_child:   None,
                        stack:        Vec::new(),
                    }));
                }
                FollowStem::KeyIsPrefix {
                    stem_step,
                } => {
                    generation.iterator_roots.insert(key)?;
                    let stem_slice = stem_iter.as_slice();
                    let mut root_key = Vec::with_capacity(key.len() + 1 + stem_slice.len());
                    root_key.extend_from_slice(key);
                    root_key.push(stem_step);
                    root_key.extend_from_slice(stem_slice);
                    return Ok(Some(Iterator {
                        root:         key.into(),
                        current_node: node_idx,
                        key:          root_key,
                        next_child:   None,
                        stack:        Vec::new(),
                    }));
                }
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    let (_, _, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    let key_usize = usize::from(key_step) << 56;
                    let pair = if let Ok(pair) = children
                        .binary_search_by(|ck| (ck.pair & 0xff00_0000_0000_0000).cmp(&key_usize))
                    {
                        pair
                    } else {
                        return Ok(None);
                    };
                    node_idx = unsafe { children.get_unchecked(pair) }.index();
                }
                FollowStem::Diff {
                    ..
                } => {
                    return Ok(None);
                }
            }
        }
    }

    /// Set the entry value to the given value. Return a mutable reference to
    /// the value if successful. This is analogous to `get_mut`, except that
    /// it avoids copying the value in case the value is currently not owned
    /// for the relevant generation.
    pub fn set(&mut self, entry: EntryId, new_value: V) -> Option<&mut V> {
        let values = &mut self.values;
        let entries = &mut self.entries;
        match entries[entry] {
            Entry::ReadOnly {
                ..
            } => {
                let value_idx = values.len();
                values.push(new_value);
                entries[entry] = Entry::Mutable {
                    entry_idx: value_idx,
                };
                values.last_mut()
            }
            Entry::Mutable {
                entry_idx,
            } => {
                values[entry_idx] = new_value;
                values.get_mut(entry_idx)
            }
            Entry::Deleted => None,
        }
    }

    /// Use the entry. This does not modify any structure.
    pub fn with_entry<X, F>(
        &self,
        entry: EntryId,
        loader: &mut impl BackingStoreLoad,
        f: F,
    ) -> Option<X>
    where
        F: FnOnce(&V) -> X,
        V: Loadable, {
        let values = &self.values;
        let borrowed_values = &self.borrowed_values;
        match self.entries[entry] {
            Entry::ReadOnly {
                borrowed,
                entry_idx,
            } => {
                if borrowed {
                    borrowed_values.get(entry_idx).map(|v| v.borrow().data.use_value(loader, f))
                } else {
                    values.get(entry_idx).map(f)
                }
            }
            Entry::Mutable {
                entry_idx,
            } => return values.get(entry_idx).map(f),
            Entry::Deleted => None,
        }
    }

    /// TODO: It might be useful to return a list of new nodes so that they
    /// may be persisted quicker than traversing the tree again.
    /// Freeze the current generation. Returns None if the tree was empty.
    pub fn freeze<Ctx: BackingStoreLoad, C: Collector<V>>(
        self,
        loader: &mut Ctx,
        collector: &mut C,
    ) -> Option<Hashed<Node<V>>>
    where
        V: ToSHA256<Ctx> + Default, {
        let mut owned_nodes = self.nodes;
        let mut values = self.values;
        let entries = self.entries;
        let mut borrowed_values = self.borrowed_values;
        let root_idx = self.generations.last()?.root?;
        // get the reachable owned nodes.
        let mut reachable_stack = vec![root_idx];
        let mut reachable = Vec::new();
        while let Some(idx) = reachable_stack.pop() {
            reachable.push(idx);
            if let Some((_, children)) = owned_nodes[idx].children.get_owned() {
                for c in children {
                    reachable_stack.push(c.index());
                }
            }
        }
        // The 'reachable' array now has all reachable nodes in the order such that
        // a child of a node is always after the node itself. The root is at the
        // beginning of the array.
        // Now traverse the nodes bottom up, right to left.
        let mut nodes = HashMap::new();
        for node_idx in reachable.into_iter().rev() {
            let node = std::mem::take(&mut owned_nodes[node_idx]);
            match node.children {
                ChildrenCow::Borrowed(children) => {
                    let value = freeze_value(
                        &mut borrowed_values,
                        &mut values,
                        &entries,
                        node.value,
                        loader,
                        collector,
                    );
                    collector.add_path(node.path.as_ref().len());
                    collector.add_children(children.len());
                    let value = Node {
                        value,
                        path: node.path,
                        children,
                    };
                    let hash = value.hash(loader);
                    nodes.insert(node_idx, Hashed::new(hash, value));
                }
                ChildrenCow::Owned {
                    value: owned,
                    ..
                } => {
                    let mut children = Vec::with_capacity(owned.len());
                    for child in owned {
                        let child_node = nodes.remove(&child.index()).unwrap();
                        children.push((
                            child.key(),
                            Link::new(CachedRef::Memory {
                                value: child_node,
                            }),
                        ));
                    }
                    let value = freeze_value(
                        &mut borrowed_values,
                        &mut values,
                        &entries,
                        node.value,
                        loader,
                        collector,
                    );
                    collector.add_path(node.path.as_ref().len());
                    collector.add_children(children.len());
                    let new_node = Node {
                        value,
                        path: node.path,
                        children,
                    };
                    let hash = new_node.hash(loader);
                    nodes.insert(node_idx, Hashed::new(hash, new_node));
                }
            }
        }
        let mut nodes_iter = nodes.into_iter();
        if let Some((_, root)) = nodes_iter.next() {
            assert!(nodes_iter.next().is_none(), "Invariant violation.");
            Some(root)
        } else {
            unreachable!("Invariant violation. Root not in the nodes map.");
        }
    }

    pub fn get_entry(
        &mut self,
        loader: &mut impl BackingStoreLoad,
        key: &[KeyPart],
    ) -> Option<EntryId> {
        let mut key_iter = key.iter();
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let entries = &mut self.entries;
        let mut node_idx = self.generations.last()?.root?;
        loop {
            let node = unsafe { owned_nodes.get_unchecked(node_idx) };
            match follow_stem(&mut key_iter, &mut node.path.as_ref().iter()) {
                FollowStem::Equal => {
                    return node.value;
                }
                FollowStem::KeyIsPrefix {
                    ..
                } => {
                    return None;
                }
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    let (_, _, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    let key_usize = usize::from(key_step) << 56;
                    let pair = children
                        .binary_search_by(|ck| (ck.pair & 0xff00_0000_0000_0000).cmp(&key_usize))
                        .ok()?;
                    node_idx = unsafe { children.get_unchecked(pair) }.index();
                }
                FollowStem::Diff {
                    ..
                } => {
                    return None;
                }
            };
        }
    }

    /// Delete the given key from the trie. If the entry is in a part of the
    /// tree that is locked this returns an error. Otherwise return whether
    /// an entry existed.
    pub fn delete(
        &mut self,
        loader: &mut impl BackingStoreLoad,
        key: &[KeyPart],
    ) -> Result<bool, AttemptToModifyLockedArea>
    where
        V: Default, {
        let mut key_iter = key.iter();
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let owned_values = &mut self.values;
        let entries = &mut self.entries;
        let mut grandfather = None;
        let mut father = None;
        let generation = if let Some(generation) = self.generations.last_mut() {
            generation
        } else {
            return Ok(false);
        };
        let mut node_idx = if let Some(node_idx) = generation.root {
            node_idx
        } else {
            return Ok(false);
        };
        generation.iterator_roots.check_has_no_prefix(key)?;
        loop {
            let node = unsafe { owned_nodes.get_unchecked_mut(node_idx) };
            match follow_stem(&mut key_iter, &mut node.path.as_ref().iter()) {
                FollowStem::Equal => {
                    // we found it, delete the value first and save it for returning.
                    let rv;
                    if let Some(entry) = std::mem::take(&mut node.value) {
                        // We mark the entry as `Deleted` such that other ids pointing to the entry
                        // are automatically invalidated.
                        let existing_entry = std::mem::replace(&mut entries[entry], Entry::Deleted);
                        // if this entry was owned we now also clean up the stored value to
                        // deallocate any storage.
                        if let Some(value_idx) = existing_entry.is_owned() {
                            std::mem::take(&mut owned_values[value_idx]);
                        }
                        rv = existing_entry.is_alive();
                    } else {
                        // no value here, so no entry was found
                        return Ok(false);
                    }
                    let (_, _, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    if children.len() == 1 {
                        // collapse path from father
                        if let Some(child) = children.pop() {
                            let node = std::mem::take(&mut owned_nodes[node_idx]); // invalidate the node.
                            let child_node = &mut owned_nodes[child.index()];
                            let mut new_stem: Stem = node.path;
                            new_stem.extend(child.key(), child_node.path.as_ref());
                            child_node.path = new_stem;
                            if let Some((child_idx, father_idx)) = father {
                                // skip the current node
                                // father's child pointer should point directly to the node's child,
                                // instead of the node.
                                // the only thing that needs to be transferred from the node to the
                                // child is (potentially) the stem of the node.
                                let father_node: &mut MutableNode<_> =
                                    unsafe { owned_nodes.get_unchecked_mut(father_idx) };
                                if let Some((_, children)) = father_node.children.get_owned_mut() {
                                    let child_place: &mut KeyIndexPair = &mut children[child_idx];
                                    let step = child_place.key();
                                    *child_place = KeyIndexPair::new(step, child.index());
                                } else {
                                    unsafe { std::hint::unreachable_unchecked() }
                                }
                            } else {
                                // set the root to the new child
                                generation.root = Some(child.index());
                            }
                        }
                    } else if children.is_empty() {
                        // no children are left, and also no value, we need to delete the child from
                        // the father.
                        if let Some((child_idx, father_idx)) = father {
                            let (has_value, _, father_children) = make_owned(
                                father_idx,
                                borrowed_values,
                                owned_nodes,
                                entries,
                                loader,
                            );
                            father_children.remove(child_idx);
                            // the father must have had
                            // - either at least two children
                            // - or a value
                            // otherwise it would have been path compressed.
                            // if it had a value there is nothing left to do. It must stay as is.
                            // if it had exactly two children we must now path-compress it
                            if !has_value && father_children.len() == 1 {
                                // collapse path from grandfather
                                if let Some(child) = father_children.pop() {
                                    let node = std::mem::take(&mut owned_nodes[father_idx]); // invalidate the node.
                                    let child_node = &mut owned_nodes[child.index()];
                                    let mut new_stem: Stem = node.path;
                                    new_stem.extend(child.key(), child_node.path.as_ref());
                                    child_node.path = new_stem;
                                    if let Some((child_idx, grandfather_idx)) = grandfather {
                                        // skip the current node
                                        // grandfather's child pointer should point directly to the
                                        // node's child, instead of the node.
                                        // the only thing that needs to be transferred from the node
                                        // to the child is (potentially) the stem of the node.
                                        let grandfather_node: &mut MutableNode<_> = unsafe {
                                            owned_nodes.get_unchecked_mut(grandfather_idx)
                                        };
                                        if let Some((_, children)) =
                                            grandfather_node.children.get_owned_mut()
                                        {
                                            let child_place: &mut KeyIndexPair =
                                                &mut children[child_idx];
                                            let step = child_place.key();
                                            *child_place = KeyIndexPair::new(step, child.index());
                                        } else {
                                            unsafe { std::hint::unreachable_unchecked() }
                                        }
                                    } else {
                                        // grandfather did not exist
                                        // set the root to the new child
                                        generation.root = Some(child.index());
                                    }
                                } else {
                                    unsafe { std::hint::unreachable_unchecked() }
                                }
                            }
                        } else {
                            // otherwise this must be the root. Delete it.
                            generation.root = None;
                        }
                    }
                    return Ok(rv);
                }
                FollowStem::KeyIsPrefix {
                    ..
                } => {
                    return Ok(false);
                }
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    let (_, _, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    if let Ok(c_idx) = children.binary_search_by(|ck| ck.key().cmp(&key_step)) {
                        let pair = unsafe { children.get_unchecked(c_idx) };
                        grandfather = std::mem::replace(&mut father, Some((c_idx, node_idx)));
                        node_idx = pair.index();
                    } else {
                        return Ok(false);
                    }
                }
                FollowStem::Diff {
                    ..
                } => {
                    return Ok(false);
                }
            };
        }
    }

    /// Delete the entire subtree whose keys match the given prefix, that is,
    /// where the given key is a prefix. Return
    /// - either an error caused by the counter
    /// - an error caused by attempting to modify a locked part of the tree
    /// - otherwise return whether anything was deleted
    pub fn delete_prefix<L: BackingStoreLoad, C: TraversalCounter>(
        &mut self,
        loader: &mut L,
        key: &[KeyPart],
        counter: &mut C,
    ) -> Result<Result<bool, AttemptToModifyLockedArea>, C::Err> {
        let mut key_iter = key.iter();
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let entries = &mut self.entries;
        let mut grandparent_idx = None;
        let mut parent_idx = None;
        let generation = if let Some(generation) = self.generations.last_mut() {
            generation
        } else {
            return Ok(Ok(false));
        };
        let mut node_idx = if let Some(idx) = generation.root {
            idx
        } else {
            return Ok(Ok(false));
        };
        if generation.iterator_roots.is_or_has_prefix(key) {
            return Ok(Err(AttemptToModifyLockedArea));
        }
        loop {
            let node = unsafe { owned_nodes.get_unchecked_mut(node_idx) };
            match follow_stem(&mut key_iter, &mut node.path.as_ref().iter()) {
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    let (_, _, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    if let Ok(c_idx) = children.binary_search_by(|ck| ck.key().cmp(&key_step)) {
                        let pair = unsafe { children.get_unchecked(c_idx) };
                        grandparent_idx =
                            std::mem::replace(&mut parent_idx, Some((c_idx, node_idx)));
                        node_idx = pair.index();
                    } else {
                        return Ok(Ok(false));
                    }
                }
                FollowStem::Diff {
                    ..
                } => {
                    return Ok(Ok(false));
                }
                _ => {
                    // We found the subtree to remove.
                    // First we check that the root of the subtree and it's children are not locked.
                    // Second, invalidate entry of the node and all of its children.
                    let mut nodes_to_invalidate = vec![node_idx];
                    // traverse each child subtree and invalidate them.
                    while let Some(node_idx) = nodes_to_invalidate.pop() {
                        let to_invalidate = &owned_nodes[node_idx];
                        counter.tick(to_invalidate.path.as_ref().len() as u64 + 1)?; // + 1 is for the step from the parent.
                        if let Some(entry) = to_invalidate.value {
                            entries[entry] = Entry::Deleted;
                        }

                        // if children are borrowed then by construction there are no entries
                        // in them. Hence we only need to recurse into owned children.
                        if let Some((generation, children)) = to_invalidate.children.get_owned() {
                            // if children are of a previous generation then, again, we
                            // do not have to recurse, since all entries will be in fully owned
                            // nodes, and that means they will be of
                            // current generation.
                            if to_invalidate.generation == generation {
                                for v in children.iter() {
                                    nodes_to_invalidate.push(v.index())
                                }
                            }
                        }
                    }
                    // Now fix up the tree. We deleted a child of the parent. If the
                    // parent now has a single remaining child and no value we must collapse it with
                    // its parent, the grandfather, if it exists. If either the father nor the
                    // grandfather exist then they are in effect the root, so we change the root
                    // pointer to point to the relevant node, or None if we
                    // deleted the entire tree.
                    if let Some((child_idx, parent_idx)) = parent_idx {
                        let (has_value, _, children) =
                            make_owned(parent_idx, borrowed_values, owned_nodes, entries, loader);

                        children.remove(child_idx);
                        // if the node does not have a value and it has one child, then it should be
                        // collapsed (path compressed)
                        if !has_value && children.len() == 1 {
                            // collapse path.
                            if let Some(child) = children.pop() {
                                let parent_node: MutableNode<_> =
                                    std::mem::take(&mut owned_nodes[parent_idx]);
                                let child_node = &mut owned_nodes[child.index()];
                                let mut new_stem: Stem = parent_node.path;
                                new_stem.extend(child.key(), child_node.path.as_ref());
                                child_node.path = new_stem;
                                if let Some((child_idx, grandparent_idx)) = grandparent_idx {
                                    // skip the parent
                                    // grandfather's child pointer should point directly to the
                                    // node's child, instead of the node.
                                    // the only thing that needs to be transferred from the node
                                    // to the child is (potentially) the stem of the node.
                                    // All other values in the node are empty.
                                    let grandparent_node: &mut MutableNode<_> =
                                        unsafe { owned_nodes.get_unchecked_mut(grandparent_idx) };
                                    if let Some((_, children)) =
                                        grandparent_node.children.get_owned_mut()
                                    {
                                        let child_place: &mut KeyIndexPair =
                                            &mut children[child_idx];
                                        let step = child_place.key();
                                        *child_place = KeyIndexPair::new(step, child.index());
                                    } else {
                                        unsafe { std::hint::unreachable_unchecked() }
                                    }
                                } else {
                                    // grandparent did not exist
                                    // set the root to the new child
                                    if let Some(generation) = self.generations.last_mut() {
                                        generation.root = Some(child.index());
                                    }
                                }
                            } else {
                                unsafe { std::hint::unreachable_unchecked() }
                            }
                        }
                    } else {
                        generation.root = None;
                        return Ok(Ok(true));
                    }
                    return Ok(Ok(true));
                }
            };
        }
    }

    /// Returns the new entry id, and potentially an existing one if the value
    /// at the key existed.
    pub fn insert(
        &mut self,
        loader: &mut impl BackingStoreLoad,
        key: &[KeyPart],
        new_value: V,
    ) -> Result<(EntryId, Option<EntryId>), AttemptToModifyLockedArea> {
        let (current_generation, older_generations) = self
            .generations
            .split_last_mut()
            .expect("There should always be at least 1 generation.");
        current_generation.iterator_roots.check_has_no_prefix(key)?;
        // if the tree is empty we must create a new root
        let mut node_idx = if let Some(root) = current_generation.root {
            root
        } else {
            // the tree is empty
            let value_idx = self.values.len();
            self.values.push(new_value);
            let generation_idx = older_generations.len() as u32;
            let root_idx = self.nodes.len();
            let entry_idx: EntryId = self.entries.len().into();
            self.entries.push(Entry::Mutable {
                entry_idx: value_idx,
            });
            self.nodes.push(MutableNode {
                generation: generation_idx,
                value:      Some(entry_idx),
                path:       key.into(),
                children:   ChildrenCow::Owned {
                    generation: generation_idx,
                    value:      tinyvec::TinyVec::new(),
                },
            });
            current_generation.root = Some(root_idx);
            return Ok((entry_idx, None));
        };
        let owned_nodes = &mut self.nodes;
        let borrowed_values = &mut self.borrowed_values;
        let entries = &mut self.entries;
        // the parent node index and the index of the parents child we're visiting.
        let mut parent_node_idxs: Option<(usize, usize)> = None;
        let generation = owned_nodes[node_idx].generation;
        let mut key_iter = key.iter();
        loop {
            let key_slice = key_iter.as_slice();
            let owned_nodes_len = owned_nodes.len();
            let node = unsafe { owned_nodes.get_unchecked_mut(node_idx) };
            let mut stem_iter = node.path.as_ref().iter();
            match follow_stem(&mut key_iter, &mut stem_iter) {
                FollowStem::Equal => {
                    let value_idx = self.values.len();
                    self.values.push(new_value);
                    let old_entry_idx = node.value;
                    // insert new entry
                    let entry_idx: EntryId = self.entries.len().into();
                    self.entries.push(Entry::Mutable {
                        entry_idx: value_idx,
                    });
                    node.value = Some(entry_idx);
                    return Ok((entry_idx, old_entry_idx));
                }
                FollowStem::KeyIsPrefix {
                    stem_step,
                } => {
                    // create a new branch with the value being the new_value since the key ends
                    // here.
                    let remaining_stem: Stem = stem_iter.as_slice().into();
                    let value_idx = self.values.len();
                    self.values.push(new_value);
                    let entry_idx: EntryId = self.entries.len().into();
                    self.entries.push(Entry::Mutable {
                        entry_idx: value_idx,
                    });

                    node.path = remaining_stem;
                    let new_node_idx = owned_nodes_len;

                    // Update the parents children index with the new child
                    if let Some((parent_node_idx, child_idx)) = parent_node_idxs {
                        let parent_node = unsafe { owned_nodes.get_unchecked_mut(parent_node_idx) };
                        if let Some((_, children)) = parent_node.children.get_owned_mut() {
                            if let Some(key_and_index) = children.get_mut(child_idx) {
                                let key = key_and_index.key();
                                *key_and_index = KeyIndexPair::new(key, new_node_idx);
                            }
                        }
                    } else {
                        current_generation.root = Some(new_node_idx);
                    }
                    let new_node = MutableNode {
                        generation,
                        value: Some(entry_idx),
                        path: key_slice.into(),
                        children: ChildrenCow::Owned {
                            generation,
                            value: tinyvec::tiny_vec![[_; INLINE_CAPACITY] => KeyIndexPair::new(stem_step, node_idx)],
                        },
                    };
                    owned_nodes.push(new_node);
                    return Ok((entry_idx, None));
                }
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    // make_owned may insert additional nodes. Hence we have to update our
                    // owned_nodes_len to make sure we have the up-to-date
                    // value.
                    let (_, owned_nodes_len, children) =
                        make_owned(node_idx, borrowed_values, owned_nodes, entries, loader);
                    let idx = children.binary_search_by(|kk| kk.key().cmp(&key_step));
                    match idx {
                        Ok(idx) => {
                            parent_node_idxs = Some((node_idx, idx));
                            node_idx = unsafe { children.get_unchecked(idx).index() };
                        }
                        Err(place) => {
                            // need to create a new node.
                            let remaining_key: Stem = key_iter.as_slice().into();
                            let new_key_node_idx = owned_nodes_len;
                            children.insert(place, KeyIndexPair::new(key_step, new_key_node_idx));
                            let value_idx = self.values.len();
                            self.values.push(new_value);
                            // insert new entry
                            let entry_idx: EntryId = entries.len().into();
                            entries.push(Entry::Mutable {
                                entry_idx: value_idx,
                            });
                            owned_nodes.push(MutableNode {
                                generation,
                                value: Some(entry_idx),
                                path: remaining_key,
                                children: ChildrenCow::Owned {
                                    generation,
                                    value: tinyvec::TinyVec::new(),
                                },
                            });
                            return Ok((entry_idx, None));
                        }
                    }
                }
                FollowStem::Diff {
                    key_step,
                    stem_step,
                } => {
                    // create a new branch with the value being the new_value since the key ends
                    // here.
                    let remaining_stem: Stem = stem_iter.as_slice().into();
                    let remaining_key_len = key_iter.as_slice().len();
                    let remaining_key: Stem = key_iter.as_slice().into();
                    let new_stem =
                        Stem::from(&key_slice[0..key_slice.len() - remaining_key_len - 1]);
                    // index of the node that continues along the remaining key
                    let remaining_key_node_idx = owned_nodes_len;
                    // index of the new node that will have two children
                    let new_node_idx = owned_nodes_len + 1;
                    node.path = remaining_stem;
                    // insert new entry
                    let value_idx = self.values.len();
                    self.values.push(new_value);
                    let entry_idx: EntryId = self.entries.len().into();
                    self.entries.push(Entry::Mutable {
                        entry_idx: value_idx,
                    });
                    {
                        let remaining_key_node = MutableNode {
                            generation,
                            value: Some(entry_idx),
                            path: remaining_key,
                            children: ChildrenCow::Owned {
                                generation: generation as u32,
                                value:      tinyvec::TinyVec::new(),
                            },
                        };
                        owned_nodes.push(remaining_key_node);
                    }

                    // construct the new node with two children
                    {
                        let children = if key_step < stem_step {
                            tinyvec::tiny_vec![
                                [_; INLINE_CAPACITY] =>
                                KeyIndexPair::new(key_step, remaining_key_node_idx),
                                KeyIndexPair::new(stem_step, node_idx),
                            ]
                        } else {
                            tinyvec::tiny_vec![
                                [_; INLINE_CAPACITY] =>
                                KeyIndexPair::new(stem_step, node_idx),
                                KeyIndexPair::new(key_step, remaining_key_node_idx),
                            ]
                        };
                        let new_node = MutableNode {
                            generation,
                            value: None,
                            path: new_stem,
                            children: ChildrenCow::Owned {
                                generation,
                                value: children,
                            },
                        };
                        owned_nodes.push(new_node);
                    }

                    // Update the parents children index with the new child
                    if let Some((parent_node_idx, child_idx)) = parent_node_idxs {
                        let parent_node = unsafe { owned_nodes.get_unchecked_mut(parent_node_idx) };
                        if let Some((_, children)) = parent_node.children.get_owned_mut() {
                            if let Some(key_and_index) = children.get_mut(child_idx) {
                                let key = key_and_index.key();
                                *key_and_index = KeyIndexPair::new(key, new_node_idx);
                            }
                        }
                    } else {
                        current_generation.root = Some(new_node_idx);
                    }
                    return Ok((entry_idx, None));
                }
            }
        }
    }
}

/// Store the node's value tag (whether the value is present or not) together
/// with the length of the stem. This should match
/// [read_node_path_and_value_tag] below.
#[inline(always)]
fn write_node_path_and_value_tag(
    stem: &[u8],
    no_value: bool,
    out: &mut impl Write,
) -> Result<(), std::io::Error> {
    let stem_len = stem.len();
    let value_mask: u8 = if no_value {
        0
    } else {
        0b0100_0000
    };
    // if the first bit is 0 then the first byte encodes
    // path length + presence of value
    if stem_len <= INLINE_STEM_LENGTH {
        let tag = stem_len as u8 | value_mask;
        out.write_u8(tag)
    } else {
        // We could optimize this as well by using variable-length encoding.
        // But it probably does not matter in practice since paths should really always
        // be < 64 in length.
        let tag = 0b1000_0000 | value_mask;
        out.write_u8(tag)?;
        out.write_u32::<BigEndian>(stem_len as u32)
    }
}

#[inline(always)]
/// Read a node path and whether the value exists. This should match
/// [write_node_path_and_value_tag] above.
fn read_node_path_and_value_tag(source: &mut impl Read) -> Result<(Stem, bool), std::io::Error> {
    let tag = source.read_u8()?;
    let path_len = if tag & 0b1000_0000 == 0 {
        // stem length is encoded in the tag
        u32::from(tag & 0b0011_1111)
    } else {
        // stem length follows as a u32
        source.read_u32::<BigEndian>()?
    };
    let mut path = vec![0u8; path_len as usize];
    source.read_exact(&mut path)?;
    let path = Stem::from(path);
    Ok((path, (tag & 0b100_0000 != 0)))
}

impl<V: AsRef<[u8]> + Loadable> Hashed<Node<V>> {
    /// Serialize the node and its children into a byte array.
    /// Note that this serializes the entire tree together with its children, so
    /// it is different from store_update which only traverses the part of
    /// the tree that is in memory.
    pub fn serialize(
        &self,
        loader: &mut impl BackingStoreLoad,
        out: &mut impl std::io::Write,
    ) -> anyhow::Result<()> {
        // this limits the tree size to 4 billion nodes.
        let mut node_counter: u32 = 0;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((self.clone(), node_counter));
        while let Some((node, idx)) = queue.pop_front() {
            out.write_u32::<BigEndian>(node_counter - idx)?;
            out.write_all(node.hash.as_ref())?;
            let node = &node.data;
            write_node_path_and_value_tag(node.path.as_ref(), node.value.is_none(), out)?;
            // store the path
            out.write_all(node.path.as_ref())?;
            // store the value
            if let Some(v) = node.value.as_ref() {
                let borrowed = v.borrow();
                out.write_all(borrowed.hash.as_ref())?;
                borrowed.data.use_value(loader, |v| {
                    out.write_u32::<BigEndian>(v.as_ref().len() as u32)?;
                    out.write_all(v.as_ref())
                })?;
            }
            out.write_u16::<BigEndian>(node.children.len() as u16)?;
            let parent_idx = node_counter;
            for (key, child) in node.children.iter() {
                out.write_u8(*key)?;
                child.borrow().use_value(loader, |nd| queue.push_back((nd.clone(), parent_idx)));
            }
            node_counter += 1;
        }
        Ok(())
    }

    /// Serialize the node and its children into a byte array.
    /// Note that this serializes the entire tree together with its children, so
    /// it is different from store_update which only traverses the part of
    /// the tree that is in memory.
    pub fn deserialize(source: &mut impl std::io::Read) -> anyhow::Result<Self>
    where
        V: From<Vec<u8>>, {
        let mut parents: Vec<Link<CachedRef<Hashed<Node<V>>>>> = Vec::new();
        let mut todo = std::collections::VecDeque::new();
        todo.push_back(0); // dummy initial value, will not be used.
        while let Some(key) = todo.pop_front() {
            let idx = source.read_u32::<BigEndian>()?;
            let hash = Hash::read(source)?;
            let (path, has_value) = read_node_path_and_value_tag(source)?;
            let value = if has_value {
                let value_hash = Hash::read(source)?;
                let value_len = source.read_u32::<BigEndian>()?;
                let mut val = vec![0u8; value_len as usize];
                source.read_exact(&mut val)?;
                Some(Link::new(Hashed::new(value_hash, CachedRef::Memory {
                    value: val.into(),
                })))
            } else {
                None
            };
            let num_children = source.read_u16::<BigEndian>()?;
            let new_node = Link::new(CachedRef::Memory {
                value: Hashed::new(hash, Node {
                    value,
                    path,
                    children: Vec::new(),
                }),
            });
            if idx > 0 {
                let mut parent = parents[parents.len() - idx as usize].borrow_mut();
                if let CachedRef::Memory {
                    value,
                } = &mut *parent
                {
                    value.data.children.push((key, new_node.clone()));
                } else {
                    // all values are allocated in this function, so in-memory.
                    unsafe { std::hint::unreachable_unchecked() };
                }
            }
            for _ in 0..num_children {
                let key = source.read_u8()?;
                todo.push_back(key);
            }
            parents.push(new_node);
        }
        if let Some(root) = parents.into_iter().next() {
            let rw = std::mem::take(&mut *root.borrow_mut());
            if let CachedRef::Memory {
                value,
            } = rw
            {
                Ok(value)
            } else {
                // all values are allocated in this function, so in-memory.
                unsafe { std::hint::unreachable_unchecked() };
            }
        } else {
            // all values are allocated in this function, so in-memory, and there is at
            // least one.
            unsafe { std::hint::unreachable_unchecked() };
        }
    }
}

/// Result of [follow_stem] below.
enum FollowStem {
    /// Iterators were equal. Both were consumed to the end.
    Equal,
    /// The key iterator is a strict prefix of the stem iterator.
    /// The first item of the stem past the key is returned.
    KeyIsPrefix {
        stem_step: KeyPart,
    },
    /// The stem iterator is a strict prefix of the key iterator.
    /// The first item of the key past the stem is returned.
    StemIsPrefix {
        key_step: KeyPart,
    },
    /// The stem and key iterators differ. The items where they differ are
    /// returned.
    Diff {
        key_step:  KeyPart,
        stem_step: KeyPart,
    },
}

#[inline(always)]
/// Given two iterators, representing the key and the stem of the node, advance
/// them stepwise until either at least one of them is exhausted or the steps
/// differ. Return which option occurred.
fn follow_stem(key_iter: &mut Iter<KeyPart>, stem_iter: &mut Iter<KeyPart>) -> FollowStem {
    for &stem_step in stem_iter {
        if let Some(&key_step) = key_iter.next() {
            if stem_step != key_step {
                return FollowStem::Diff {
                    key_step,
                    stem_step,
                };
            }
        } else {
            // key is a prefix of stem
            return FollowStem::KeyIsPrefix {
                stem_step,
            };
        }
    }
    if let Some(&key_step) = key_iter.next() {
        FollowStem::StemIsPrefix {
            key_step,
        }
    } else {
        FollowStem::Equal
    }
}

impl<V: Clone> Node<V> {
    /// TODO: This is not very efficient. It involves cloning nodes, which is
    /// not all that cheap.
    /// We also don't need this in production, so it is low priority to fix.
    pub fn lookup(
        &self,
        loader: &mut impl BackingStoreLoad,
        key: &[KeyPart],
    ) -> Option<Link<Hashed<CachedRef<V>>>> {
        let mut key_iter = key.iter();
        let mut path = self.path.as_ref().to_vec();
        let mut children = self.children.clone();
        let mut value = self.value.clone();
        let mut tmp = Vec::new();
        loop {
            match follow_stem(&mut key_iter, &mut path.iter()) {
                FollowStem::Equal => {
                    return value;
                }
                FollowStem::KeyIsPrefix {
                    ..
                } => {
                    return None;
                }
                FollowStem::StemIsPrefix {
                    key_step,
                } => {
                    let (_, c) = children.iter().find(|&&(ck, _)| ck == key_step)?;
                    c.borrow().use_value(loader, |node| {
                        path.clear();
                        path.extend_from_slice(node.data.path.as_ref());
                        tmp.clear();
                        tmp.extend_from_slice(&node.data.children);
                        value = node.data.value.clone();
                    });
                    children.clear();
                    children.append(&mut tmp);
                }
                FollowStem::Diff {
                    ..
                } => {
                    return None;
                }
            }
        }
    }

    /// Check that the node is stored, that is, that its value and
    /// children are already stored in persistent storage, and possibly in
    /// memory.
    pub fn is_stored(&self) -> bool {
        if let Some(value) = &self.value {
            if let CachedRef::Memory {
                ..
            } = value.borrow().data
            {
                return false;
            }
        }
        for child in self.children.iter() {
            if let CachedRef::Memory {
                ..
            } = &*child.1.borrow()
            {
                return false;
            }
        }
        true
    }

    /// Check that the entire tree is cached, meaning that it is in memory,
    /// either purely in memory or on disk and in memory.
    /// WARNING: Note that this method is recursive, and thus should only be
    /// used for small trees.
    pub fn is_cached(&self) -> bool {
        if let Some(value) = &self.value {
            if let CachedRef::Disk {
                ..
            } = value.borrow().data
            {
                return false;
            }
        }
        for child in self.children.iter() {
            match &*child.1.borrow() {
                CachedRef::Disk {
                    key: _,
                } => {
                    return false;
                }
                CachedRef::Memory {
                    value,
                } => {
                    if !value.data.is_cached() {
                        return false;
                    }
                }
                CachedRef::Cached {
                    key: _,
                    value,
                } => {
                    if !value.data.is_cached() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
/// Tests for the prefix map.
mod prefix_map_tests {
    use super::PrefixesMap;
    const NUM_TESTS: u64 = 100000;
    #[test]
    fn prop_insert_delete() {
        let prop = |keys: Vec<Vec<u8>>| -> anyhow::Result<()> {
            let mut map = PrefixesMap::new();
            for key in keys.iter() {
                if map.insert(key).is_err() {
                    // ignore tests which cause overflow
                    return Ok(());
                }
            }
            for key in keys.iter() {
                anyhow::ensure!(map.delete(key), "Every inserted key should be deleted.");
            }
            anyhow::ensure!(map.is_empty(), "Deleting everything should leave the map empty.");
            anyhow::ensure!(map.nodes.is_empty(), "Slab should be empty.");
            Ok(())
        };
        quickcheck::QuickCheck::new()
            .tests(NUM_TESTS)
            .quickcheck(prop as fn(_) -> anyhow::Result<()>);
    }

    #[test]
    fn prop_is_prefix() {
        let prop = |keys: Vec<Vec<u8>>, prefixes: Vec<Vec<u8>>| -> anyhow::Result<()> {
            let mut map = PrefixesMap::new();
            for key in keys.iter() {
                // ignore tests which cause overflow
                if map.insert(key).is_err() {
                    return Ok(());
                }
            }
            for prefix in prefixes.iter() {
                let has_any_prefix = keys.iter().any(|key| prefix.starts_with(key));
                let res = map.check_has_no_prefix(prefix);
                anyhow::ensure!(
                    has_any_prefix == res.is_err(),
                    "Reference ({}) differs from actual ({:?}).",
                    has_any_prefix,
                    res
                );
            }
            Ok(())
        };
        quickcheck::QuickCheck::new()
            .tests(NUM_TESTS)
            .quickcheck(prop as fn(_, _) -> anyhow::Result<()>);
    }

    #[test]
    fn prop_has_prefix() {
        let prop = |keys: Vec<Vec<u8>>, prefixes: Vec<Vec<u8>>| -> anyhow::Result<()> {
            let mut map = PrefixesMap::new();
            for key in keys.iter() {
                // ignore tests which cause overflow
                if map.insert(key).is_err() {
                    return Ok(());
                }
            }
            for prefix in prefixes.iter() {
                let has_any_as_prefix =
                    keys.iter().any(|key| key.starts_with(prefix) || prefix.starts_with(key));
                let res = map.is_or_has_prefix(prefix);
                anyhow::ensure!(
                    has_any_as_prefix == res,
                    "Reference ({}) differs from actual ({}).",
                    has_any_as_prefix,
                    res
                );
            }
            Ok(())
        };
        quickcheck::QuickCheck::new()
            .tests(NUM_TESTS)
            .quickcheck(prop as fn(_, _) -> anyhow::Result<()>);
    }
}
