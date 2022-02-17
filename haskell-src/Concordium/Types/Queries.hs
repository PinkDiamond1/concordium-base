{-# LANGUAGE DataKinds #-}
{-# LANGUAGE ExistentialQuantification #-}
{-# LANGUAGE GADTs #-}
{-# LANGUAGE KindSignatures #-}
{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE RankNTypes #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE TemplateHaskell #-}
{-# LANGUAGE TypeApplications #-}

-- |Types for representing the results of consensus queries.
module Concordium.Types.Queries where

import Data.Aeson
import Data.Aeson.TH
import Data.Aeson.Types (Parser)
import Data.Char (isLower)
import qualified Data.Map as Map
import qualified Data.Sequence as Seq
import Data.Time (UTCTime)
import qualified Data.Vector as Vec
import Data.Word

import Concordium.Types
import Concordium.Types.Accounts
import Concordium.Types.Block
import Concordium.Types.Execution (TransactionSummary)
import Concordium.Types.Transactions (SpecialTransactionOutcome)
import Concordium.Types.UpdateQueues (Updates)
import Concordium.Utils

-- |Result type for @getConsensusStatus@ queries.  A number of fields are not defined when no blocks
-- have so far been received, verified or finalized. In such cases, the values will be 'Nothing'.
--
-- The JSON serialization of this structure is as an object, with fields named as in the record,
-- but without the "cs" prefix, and the first letter in lowercase.
data ConsensusStatus = ConsensusStatus
    { -- |Hash of the current best block
      csBestBlock :: !BlockHash,
      -- |Hash of the (original) genesis block
      csGenesisBlock :: !BlockHash,
      -- |Time of the (original) genesis block
      csGenesisTime :: !UTCTime,
      -- |(Current) slot duration in milliseconds
      csSlotDuration :: !Duration,
      -- |(Current) epoch duration in milliseconds
      csEpochDuration :: !Duration,
      -- |Hash of the last finalized block
      csLastFinalizedBlock :: !BlockHash,
      -- |Absolute height of the best block
      csBestBlockHeight :: !AbsoluteBlockHeight,
      -- |Absolute height of the last finalized block
      csLastFinalizedBlockHeight :: !AbsoluteBlockHeight,
      -- |Total number of blocks received
      csBlocksReceivedCount :: !Int,
      -- |The last time a block was received
      csBlockLastReceivedTime :: !(Maybe UTCTime),
      -- |Moving average latency between a block's slot time and received time
      csBlockReceiveLatencyEMA :: !Double,
      -- |Standard deviation of moving average latency between a block's slot time and received time
      csBlockReceiveLatencyEMSD :: !Double,
      -- |Moving average time between receiving blocks
      csBlockReceivePeriodEMA :: !(Maybe Double),
      -- |Standard deviation of moving average time between receiving blocks
      csBlockReceivePeriodEMSD :: !(Maybe Double),
      -- |Total number of blocks received and verified
      csBlocksVerifiedCount :: !Int,
      -- |The last time a block was verified (added to the tree)
      csBlockLastArrivedTime :: !(Maybe UTCTime),
      -- |Moving average latency between a block's slot time and its arrival
      csBlockArriveLatencyEMA :: !Double,
      -- |Standard deviation of moving average latency between a block's slot time and its arrival
      csBlockArriveLatencyEMSD :: !Double,
      -- |Moving average time between block arrivals
      csBlockArrivePeriodEMA :: !(Maybe Double),
      -- |Standard deviation of moving average time between block arrivals
      csBlockArrivePeriodEMSD :: !(Maybe Double),
      -- |Moving average number of transactions per block
      csTransactionsPerBlockEMA :: !Double,
      -- |Standard deviation of moving average number of transactions per block
      csTransactionsPerBlockEMSD :: !Double,
      -- |Number of finalizations
      csFinalizationCount :: !Int,
      -- |Time of last verified finalization
      csLastFinalizedTime :: !(Maybe UTCTime),
      -- |Moving average time between finalizations
      csFinalizationPeriodEMA :: !(Maybe Double),
      -- |Standard deviation of moving average time between finalizations
      csFinalizationPeriodEMSD :: !(Maybe Double),
      -- |Currently active protocol version.
      csProtocolVersion :: !ProtocolVersion,
      -- |The number of chain restarts via a protocol update. An effected
      -- protocol update instruction might not change the protocol version
      -- specified in the previous field, but it always increments the genesis
      -- index.
      csGenesisIndex :: !GenesisIndex,
      -- |Block hash of the genesis block of current era, i.e., since the last protocol update.
      -- Initially this is equal to 'csGenesisBlock'.
      csCurrentEraGenesisBlock :: !BlockHash,
      -- |Time when the current era started.
      csCurrentEraGenesisTime :: !UTCTime
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''ConsensusStatus)

-- |Result type for @getBranches@ query. A 'Branch' consists of the hash of a block and 'Branch'es
-- for each child of the block.
data Branch = Branch
    { -- |Block hash
      branchBlockHash :: !BlockHash,
      -- |Child branches
      branchChildren :: ![Branch]
    }
    deriving (Eq, Ord, Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''Branch)

-- |Result type for @getNextAccountNonce@ query.
-- If all account transactions are finalized then this information is reliable.
-- Otherwise this is the best guess, assuming all other transactions will be
-- committed to blocks and eventually finalized.
data NextAccountNonce = NextAccountNonce
    { -- |The next account nonce
      nanNonce :: !Nonce,
      -- |True if all transactions on the account are finalized
      nanAllFinal :: !Bool
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''NextAccountNonce)

-- |Result type for @getBlockInfo@ query.
data BlockInfo = BlockInfo
    { -- |The block hash
      biBlockHash :: !BlockHash,
      -- |The parent block hash. For a re-genesis block, this will be the terminal block of the
      -- previous chain. For the initial genesis block, this will be the hash of the block itself.
      biBlockParent :: !BlockHash,
      -- |The last finalized block when this block was baked
      biBlockLastFinalized :: !BlockHash,
      -- |The height of this block
      biBlockHeight :: !AbsoluteBlockHeight,
      -- |The genesis index for this block. This counts the number of protocol updates that have
      -- preceded this block, and defines the era of the block.
      biGenesisIndex :: !GenesisIndex,
      -- |The height of this block relative to the (re)genesis block of its era.
      biEraBlockHeight :: !BlockHeight,
      -- |The time the block was received
      biBlockReceiveTime :: !UTCTime,
      -- |The time the block was verified
      biBlockArriveTime :: !UTCTime,
      -- |The slot number in which the block was baked
      biBlockSlot :: !Slot,
      -- |The time of the slot in which the block was baked
      biBlockSlotTime :: !UTCTime,
      -- |The identifier of the block baker, or @Nothing@ for a
      -- genesis block.
      biBlockBaker :: !(Maybe BakerId),
      -- |Whether the block is finalized
      biFinalized :: !Bool,
      -- |The number of transactions in the block
      biTransactionCount :: !Int,
      -- |The energy cost of the transaction in the block
      biTransactionEnergyCost :: !Energy,
      -- |The size of the transactions
      biTransactionsSize :: !Int,
      -- |The hash of the block state
      biBlockStateHash :: !StateHash
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''BlockInfo)

-- |Details of a party in a finalization.
data FinalizationSummaryParty = FinalizationSummaryParty
    { -- |The identity of the baker
      fspBakerId :: !BakerId,
      -- |The party's relative weight in the committee
      fspWeight :: !Integer,
      -- |Whether the party's signature is present
      fspSigned :: !Bool
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''FinalizationSummaryParty)

-- |Details of a finalization record.
data FinalizationSummary = FinalizationSummary
    { -- |Hash of the finalized block
      fsFinalizationBlockPointer :: !BlockHash,
      -- |Index of the finalization
      fsFinalizationIndex :: !FinalizationIndex,
      -- |Finalization delay value
      fsFinalizationDelay :: !BlockHeight,
      -- |The finalization committee
      fsFinalizers :: !(Vec.Vector FinalizationSummaryParty)
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''FinalizationSummary)

-- |Detailed information about a block.
data BlockSummary = forall pv.
      IsProtocolVersion pv =>
    BlockSummary
    { -- |Details of transactions in the block
      bsTransactionSummaries :: !(Vec.Vector TransactionSummary),
      -- |Details of special events in the block
      bsSpecialEvents :: !(Seq.Seq SpecialTransactionOutcome),
      -- |Details of the finalization record in the block (if any)
      bsFinalizationData :: !(Maybe FinalizationSummary),
      -- |Details of the update queues and chain parameters as of the block
      bsUpdates :: !(Updates pv),
      -- |Protocol version proxy
      bsProtocolVersion :: SProtocolVersion pv
    }

-- |Get 'Updates' from 'BlockSummary', with continuation to avoid "escaped type variables".
{-# INLINE bsWithUpdates #-}
bsWithUpdates :: BlockSummary -> (forall pv. IsProtocolVersion pv => SProtocolVersion pv -> Updates pv -> a) -> a
bsWithUpdates BlockSummary{..} = \k -> k bsProtocolVersion bsUpdates

instance Show BlockSummary where
    showsPrec prec BlockSummary{..} = do
        showParen (prec > 11) $
            showString "BlockSummary "
                . showString " {bsTransactionSummaries = "
                . showsPrec 0 bsTransactionSummaries
                . showString ",bsSpecialEvents = "
                . showsPrec 0 bsSpecialEvents
                . showString ",bsFinalizationData = "
                . showsPrec 0 bsFinalizationData
                . showString ",bsUpdates = "
                . showsPrec 0 bsUpdates
                . showString ",bsProtocolVersion = "
                . showsPrec 0 (demoteProtocolVersion bsProtocolVersion)
                . showString "}"

instance ToJSON BlockSummary where
    toJSON BlockSummary{..} =
        object (commonFields ++ versionField)
      where
        commonFields =
            [ "transactionSummaries" .= bsTransactionSummaries,
              "specialEvents" .= bsSpecialEvents,
              "finalizationData" .= bsFinalizationData,
              "updates" .= bsUpdates
            ]

        -- The version field has been added in protocol version 4. For backwards compatibility,
        -- we will not include that field in JSON for versions < 4.
        versionField = case bsProtocolVersion of
            SP1 -> []
            SP2 -> []
            SP3 -> []
            SP4 -> ["protocolVersion" .= demoteProtocolVersion bsProtocolVersion]

instance FromJSON BlockSummary where
    parseJSON =
        withObject "BlockSummary" $ \v -> do
            -- We have added the "protocolVersion" field in protocol version 4, so in order to parse
            -- blocks summaries from older protocols, we allow this field to not exist. If the field
            -- does not exist, then we proceed like the previous protocol (version 3).
            mpv <- v .:? "protocolVersion"
            case mpv of
                Nothing -> parse (promoteProtocolVersion P3) v
                Just pv -> parse (promoteProtocolVersion pv) v
      where
        parse :: SomeProtocolVersion -> Object -> Parser BlockSummary
        parse (SomeProtocolVersion (spv :: SProtocolVersion pv)) v =
            BlockSummary
                <$> v .: "transactionSummaries"
                <*> v .: "specialEvents"
                <*> v .: "finalizationData"
                <*> (v .: "updates" :: Parser (Updates pv))
                <*> pure spv

data RewardStatus = RewardStatus
    { -- |The total GTU in existence
      rsTotalAmount :: !Amount,
      -- |The total GTU in encrypted balances
      rsTotalEncryptedAmount :: !Amount,
      -- |The amount in the baking reward account
      rsBakingRewardAccount :: !Amount,
      -- |The amount in the finalization reward account
      rsFinalizationRewardAccount :: !Amount,
      -- |The amount in the GAS account
      rsGasAccount :: !Amount
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''RewardStatus)

-- |Summary of a baker.
data BakerSummary = BakerSummary
    { -- |Baker ID
      bsBakerId :: !BakerId,
      -- |(Approximate) lottery power
      bsBakerLotteryPower :: !Double,
      -- |Baker account (should never be @Nothing@)
      bsBakerAccount :: !(Maybe AccountAddress)
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''BakerSummary)

-- |Summary of the birk parameters applicable to a particular block.
data BlockBirkParameters = BlockBirkParameters
    { -- |Baking lottery election difficulty
      bbpElectionDifficulty :: !ElectionDifficulty,
      -- |Current leadership election nonce for the lottery
      bbpElectionNonce :: !LeadershipElectionNonce,
      -- |List of the currently eligible bakers
      bbpBakers :: !(Vec.Vector BakerSummary)
    }
    deriving (Show)

$(deriveJSON defaultOptions{fieldLabelModifier = firstLower . dropWhile isLower} ''BlockBirkParameters)

-- |The status of a transaction that is present in the transaction table.
data TransactionStatus
    = -- |Transaction was received but is not in any blocks
      Received
    | -- |Transaction was received and is present in some (non-finalized) block(s)
      Committed (Map.Map BlockHash (Maybe TransactionSummary))
    | -- |Transaction has been finalized in a block
      Finalized BlockHash (Maybe TransactionSummary)
    deriving (Show)

instance ToJSON TransactionStatus where
    toJSON Received = object ["status" .= String "received"]
    toJSON (Committed m) =
        object
            [ "status" .= String "committed",
              "outcomes" .= m
            ]
    toJSON (Finalized bh outcome) =
        object
            [ "status" .= String "finalized",
              "outcomes" .= Map.singleton bh outcome
            ]

-- |The status of a transaction with respect to a specified block
data BlockTransactionStatus
    = -- |Either the transaction is not in that block, or that block is not live
      BTSNotInBlock
    | -- |The transaction was received but not known to be in that block
      BTSReceived
    | -- |The transaction is in that (non-finalized) block
      BTSCommitted (Maybe TransactionSummary)
    | -- |The transaction is in that (finalized) block
      BTSFinalized (Maybe TransactionSummary)
    deriving (Show)

instance ToJSON BlockTransactionStatus where
    toJSON BTSNotInBlock = Null
    toJSON BTSReceived = object ["status" .= String "received"]
    toJSON (BTSCommitted outcome) =
        object
            [ "status" .= String "committed",
              "result" .= outcome
            ]
    toJSON (BTSFinalized outcome) =
        object
            [ "status" .= String "finalized",
              "result" .= outcome
            ]

data PoolPendingChange
    = -- |No change is pending.
      PPCNoChange
    | -- |A reduction in baker equity capital is pending.
      PPCReduceBakerCapital
        { -- |New baker equity capital.
          ppcBakerEquityCapital :: !Amount,
          -- |Effective time of the change.
          ppcEffectiveTime :: !UTCTime
        }
    | -- |Removal of the pool is pending.
      PPCRemovePool
        { -- |Effective time of the change.
          ppcEffectiveTime :: !UTCTime
        }
    deriving (Eq, Show)

$( deriveJSON
    defaultOptions
        { fieldLabelModifier = firstLower . dropWhile isLower,
          constructorTagModifier = drop 3,
          sumEncoding =
            TaggedObject
                { tagFieldName = "pendingChangeType",
                  contentsFieldName = "pendingChangeDetails"
                }
        }
    ''PoolPendingChange
 )

-- |Construct a 'PoolPendingChange' from the 'StakePendingChange' of the pool owner.
makePoolPendingChange ::
    -- |Pool owner's pending stake change
    StakePendingChange 'AccountV1 ->
    PoolPendingChange
makePoolPendingChange NoChange = PPCNoChange
makePoolPendingChange (ReduceStake ppcBakerEquityCapital (PendingChangeEffectiveV1 et)) =
    PPCReduceBakerCapital{..}
    where
        ppcEffectiveTime = timestampToUTCTime et
makePoolPendingChange (RemoveStake (PendingChangeEffectiveV1 et)) = PPCRemovePool{..}
    where
        ppcEffectiveTime = timestampToUTCTime et

data CurrentPaydayBakerPoolStatus = CurrentPaydayBakerPoolStatus
    { -- |The number of blocks baked in the current reward period.
      bpsBlocksBaked :: !Word64,
      -- |Whether the baker has contributed a finalization proof in the current reward period.
      bpsFinalizationLive :: !Bool,
      -- |The transaction fees accruing to the pool in the current reward period.
      bpsTransactionFeesEarned :: !Amount,
      -- |The effective stake of the baker in the current reward period.
      bpsEffectiveStake :: !Amount,
      -- |The lottery power of the baker in the current reward period.
      bpsLotteryPower :: !Double,
      -- |The effective equity capital of the baker for the current reward period.
      bpsBakerEquityCapital :: !Amount,
      -- |The effective delegated capital to the pool for the current reward period.
      bpsDelegatedCapital :: !Amount
    }
    deriving (Eq, Show)

$( deriveJSON
    defaultOptions
        { fieldLabelModifier = firstLower . dropWhile isLower
        }
    ''CurrentPaydayBakerPoolStatus
 )

-- |Status information about a given pool.
--
-- Commission rates for the L-pool provide a basis for comparison with baking pools, however,
-- whereas the commission for baking pools is paid to the pool owner, "commission" is not paid
-- to anyone.  Rather, it is used to determine the level of rewards paid to delegators, so that
-- their earnings are commensurate to delegating to a baking pool with the same commission rates.
data PoolStatus
    = BakerPoolStatus
        { -- |The 'BakerId' of the pool owner.
          psBakerId :: !BakerId,
          -- |The account address of the pool owner.
          psBakerAddress :: !AccountAddress,
          -- |The equity capital provided by the pool owner.
          psBakerEquityCapital :: !Amount,
          -- |The capital delegated to the pool by other accounts.
          psDelegatedCapital :: !Amount,
          -- |The maximum amount that may be delegated to the pool, accounting for leverage and
          -- stake limits.
          psDelegatedCapitalCap :: !Amount,
          -- |The pool info associated with the pool: open status, metadata URL and commission rates.
          psPoolInfo :: !BakerPoolInfo,
          -- |Any pending change to the baker's stake.
          psBakerStakePendingChange :: !PoolPendingChange,
          -- |Status of the pool in the current reward period.
          psCurrentPaydayStatus :: !(Maybe CurrentPaydayBakerPoolStatus)
        }
    | LPoolStatus
        { -- |The total capital delegated to the L-pool.
          psDelegatedCapital :: !Amount,
          -- |The L-pool commission rates.
          psCommissionRates :: !CommissionRates,
          -- |The transaction fees accruing to the L-pool in the current reward period.
          psCurrentPaydayTransactionFeesEarned :: !Amount,
          -- |The effective delegated capital to the L-pool for the current reward period.
          psCurrentPaydayDelegatedCapital :: !Amount
        }
    deriving (Eq, Show)

$( deriveJSON
    defaultOptions
        { fieldLabelModifier = firstLower . dropWhile isLower,
          constructorTagModifier = reverse . drop (length ("Status" :: String)) . reverse,
          sumEncoding = TaggedObject{tagFieldName = "poolType", contentsFieldName = "poolStatus"}
        }
    ''PoolStatus
 )
