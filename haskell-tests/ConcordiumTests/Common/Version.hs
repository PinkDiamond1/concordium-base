{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE LambdaCase #-}
{-# LANGUAGE GeneralizedNewtypeDeriving, RecordWildCards, OverloadedStrings, LambdaCase #-}
{-# LANGUAGE TypeFamilies, ExistentialQuantification, FlexibleContexts, DeriveGeneric, DerivingVia, DeriveDataTypeable #-}
module ConcordiumTests.Common.Version where

import Concordium.Common.Version
import qualified Data.ByteString as BS
import qualified Data.Serialize as S

import Test.QuickCheck
import Test.Hspec
import Data.Aeson

testVersionTestVector :: Property
testVersionTestVector = S.encode (Version 1700794014) === (BS.pack [0x86, 0xab, 0x80, 0x9d, 0x1e])

testSerialize :: Property
testSerialize = forAll genVersion $ \v -> S.decode (S.encode v) === Right v

testJSON :: Property
testJSON = forAll genVersion $ \v -> decode (encode v) === Just v

genVersion :: Gen Version
genVersion = Version <$> arbitrary

tests :: Spec
tests = describe "Concordium.Common" $ do
  specify "versioning test vector" $ testVersionTestVector
  it "versioning binary encoding" $ withMaxSuccess 1000 $ testSerialize
  it "versioning json encoding" $ withMaxSuccess 1000 $ testJSON

