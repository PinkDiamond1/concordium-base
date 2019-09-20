{-# LANGUAGE ScopedTypeVariables #-}
module ConcordiumTests.ID.Types where

import Concordium.ID.Types
import qualified Data.FixedByteString as FBS

import Test.QuickCheck
import Test.Hspec
import Data.Aeson

testJSON :: Property
testJSON = forAll genAddress ck
  where ck :: AccountAddress -> Property
        ck b58 = case decode (encode b58) of
                   Nothing -> counterexample (show b58) False
                   Just x -> x === b58

genAddress :: Gen AccountAddress
genAddress = do
  AccountAddress . FBS.pack <$> vector accountAddressSize

tests :: Spec
tests = describe "Concordium.ID" $ do
  specify "account address JSON" $ withMaxSuccess 100000 testJSON
