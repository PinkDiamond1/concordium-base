{-| This module provides the necessary primitives for encrypted amount transfers. -}
{-# LANGUAGE DerivingStrategies, DerivingVia, StandaloneDeriving #-}
module Concordium.Crypto.EncryptedTransfers where 

import Data.Serialize
import Data.Word
import Data.Aeson
import Data.ByteString.Short
import Foreign.Ptr

import Concordium.Crypto.FFIDataTypes
import Concordium.Crypto.ByteStringHelpers

-- | Aggregate two encrypted amounts together.
foreign import ccall unsafe "aggregate_encrypted_amounts"
  aggregate_encrypted_amounts ::
     Ptr ElgamalCipher -> -- ^ High chunk of the first amount.
     Ptr ElgamalCipher -> -- ^ Low chunk of the first amount.
     Ptr ElgamalCipher -> -- ^ High chunk of the second amount.
     Ptr ElgamalCipher -> -- ^ Low chunk of the second amount.
     Ptr (Ptr ElgamalCipher) -> -- ^Place to write the pointer to the high chunk of the result.
     Ptr (Ptr ElgamalCipher) -> -- ^Place to write the pointer to the low chunk of the result.
     IO ()

data EncryptedAmount = EncryptedAmount{
  -- | Encryption of the high-chunk (highest 32 bits).
  encryptionHi :: ElgamalCipher,
  -- | Encryption of the high-chunk (lowest 32 bits).
  encryptionLow :: ElgamalCipher
  }
  deriving (FromJSON, ToJSON) via Base16JSONSerialize EncryptedAmount

instance Serialize EncryptedAmount where
  put EncryptedAmount{..} = put encryptionHi <> put encryptionLow
  get = do
    encryptionHi <- get
    encryptionLow <- get
    return EncryptedAmount{..}

-- |An indexed used to determine which encryped amounts were used in a transaction.
newtype EncryptedAmountAggIndex = EncryptedAmountAggIndex Word64
    deriving newtype (Eq, Show, Ord, FromJSON, ToJSON, Num, Integral, Real, Enum)

instance Serialize EncryptedAmountAggIndex where
  put (EncryptedAmountAggIndex i) = putWord64be i
  get = EncryptedAmountAggIndex <$> getWord64be

-- FIXME: Serialization here is probably wrong, and needs to be fixed once the proof
-- is known.
newtype EncryptedAmountTransferProof = EncryptedAmountTransferProof ShortByteString
    deriving(Eq, Show, FromJSON, ToJSON) via ByteStringHex
    deriving Serialize via Short65K

-- FIXME: Serialization here is probably wrong, and needs to be fixed once the proof
-- is known.
newtype EncryptAmountProof = EncryptAmountProof ShortByteString
    deriving(Eq, Show, FromJSON, ToJSON) via ByteStringHex
    deriving Serialize via Short65K

-- FIXME: Serialization here is probably wrong, and needs to be fixed once the proof
-- is known.
newtype DecryptAmountProof = DecryptAmountProof ShortByteString
    deriving(Eq, Show, FromJSON, ToJSON) via ByteStringHex
    deriving Serialize via Short65K
