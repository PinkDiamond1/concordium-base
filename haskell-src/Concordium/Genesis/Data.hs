{-# LANGUAGE DataKinds #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE TypeApplications #-}
{-# LANGUAGE TypeFamilies #-}

module Concordium.Genesis.Data (
    GenesisBaker (..),
    GenesisAccount (..),
    module Concordium.Genesis.Data.Base,
    module Concordium.Genesis.Data,
) where

import Data.Function (on)
import Data.Serialize

import Concordium.Genesis.Account
import Concordium.Genesis.Data.Base
import qualified Concordium.Genesis.Data.P1 as P1
import Concordium.Types

-- |Data family for genesis data.
-- This has been chosen to be a data family so that the genesis data
-- will uniquely determine the protocol version.
data family GenesisData (pv :: ProtocolVersion)

newtype instance GenesisData 'P1 = GDP1 {unGDP1 :: P1.GenesisDataP1}
newtype instance GenesisData 'P2 = GDP2 {unGDP2 :: P1.GenesisDataP1}

instance (IsProtocolVersion pv) => BasicGenesisData (GenesisData pv) where
    gdGenesisTime = case protocolVersion @pv of
        SP1 -> gdGenesisTime . unGDP1
        SP2 -> gdGenesisTime . unGDP2
    {-# INLINE gdGenesisTime #-}
    gdSlotDuration = case protocolVersion @pv of
        SP1 -> gdSlotDuration . unGDP1
        SP2 -> gdSlotDuration . unGDP2
    {-# INLINE gdSlotDuration #-}
    gdMaxBlockEnergy = case protocolVersion @pv of
        SP1 -> gdMaxBlockEnergy . unGDP1
        SP2 -> gdMaxBlockEnergy . unGDP2
    {-# INLINE gdMaxBlockEnergy #-}
    gdFinalizationParameters = case protocolVersion @pv of
        SP1 -> gdFinalizationParameters . unGDP1
        SP2 -> gdFinalizationParameters . unGDP2
    {-# INLINE gdFinalizationParameters #-}
    gdEpochLength = case protocolVersion @pv of
        SP1 -> gdEpochLength . unGDP1
        SP2 -> gdEpochLength . unGDP2
    {-# INLINE gdEpochLength #-}

instance (IsProtocolVersion pv) => Eq (GenesisData pv) where
    (==) = case protocolVersion @pv of
        SP1 -> (==) `on` unGDP1
        SP2 -> (==) `on` unGDP2

instance (IsProtocolVersion pv) => Serialize (GenesisData pv) where
    get = case protocolVersion @pv of
        SP1 -> GDP1 <$> P1.getGenesisDataV3
        SP2 -> GDP2 <$> P1.getGenesisDataV3
    put = case protocolVersion @pv of
        SP1 -> P1.putGenesisDataV3 . unGDP1
        SP2 -> P1.putGenesisDataV3 . unGDP2

-- |Deserialize genesis data with a version tag.
getVersionedGenesisData :: forall pv. IsProtocolVersion pv => Get (GenesisData pv)
getVersionedGenesisData = case protocolVersion @pv of
    SP1 -> GDP1 <$> P1.getVersionedGenesisData
    SP2 -> GDP2 <$> P1.getVersionedGenesisData

-- |Serialize genesis data with a version tag.
putVersionedGenesisData :: forall pv. IsProtocolVersion pv => Putter (GenesisData pv)
putVersionedGenesisData = case protocolVersion @pv of
    SP1 -> P1.putVersionedGenesisData . unGDP1
    SP2 -> P1.putVersionedGenesisData . unGDP2

-- |Generate the block hash of a genesis block with the given genesis data.
-- This is based on the presumption that a block hash is computed from a byte string
-- beginning with the serialization of the block slot.
genesisBlockHash :: forall pv. IsProtocolVersion pv => GenesisData pv -> BlockHash
genesisBlockHash = case protocolVersion @pv of
    SP1 -> P1.genesisBlockHash . unGDP1
    SP2 -> P1.genesisBlockHash . unGDP2
