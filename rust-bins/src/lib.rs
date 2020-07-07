use crypto_common::*;
use curve_arithmetic::*;
use id::{ffi::*, types::*};
use pairing::bls12_381::Bls12;
use serde::{de::DeserializeOwned, Serialize as SerdeSerialize};
use serde_json::{to_string_pretty, to_writer_pretty};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader},
    path::Path,
    str::FromStr,
};

pub type ExampleCurve = <Bls12 as Pairing>::G1;

pub type ExampleAttribute = AttributeKind;

pub type ExampleAttributeList = AttributeList<<Bls12 as Pairing>::ScalarField, ExampleAttribute>;

pub static GLOBAL_CONTEXT: &str = "database/global.json";
pub static IDENTITY_PROVIDERS: &str = "database/identity_providers.json";

pub fn read_global_context<P: AsRef<Path> + std::fmt::Debug>(
    filename: P,
) -> Option<GlobalContext<ExampleCurve>> {
    read_exact_versioned_json_from_file(VERSION_GLOBAL_PARAMETERS, filename).ok()
}

pub fn read_identity_providers<P: AsRef<Path> + std::fmt::Debug>(
    filename: P,
) -> Option<Vec<IpInfo<Bls12>>> {
    read_exact_versioned_vec_json_from_file(VERSION_IP_INFO_PUBLIC, filename).ok()
}

pub fn read_anonymity_revokers<P: AsRef<Path> + std::fmt::Debug>(
    filename: P,
) -> Option<BTreeMap<ArIdentity, ArInfo<ExampleCurve>>> {
    read_json_from_file(filename).ok()
}

/// Parse YYYYMM as YearMonth
pub fn parse_yearmonth(input: &str) -> Option<YearMonth> { YearMonth::from_str(input).ok() }

pub fn write_json_to_file<P: AsRef<Path>, T: SerdeSerialize>(filepath: P, v: &T) -> io::Result<()> {
    let file = File::create(filepath)?;
    Ok(to_writer_pretty(file, v)?)
}

/// Output json to standard output.
pub fn output_json<T: SerdeSerialize>(v: &T) {
    println!("{}", to_string_pretty(v).unwrap());
}

pub fn read_json_from_file<P, T>(path: P) -> io::Result<T>
where
    P: AsRef<Path> + std::fmt::Debug,
    T: DeserializeOwned, {
    let file = File::open(path)?;

    let reader = BufReader::new(file);
    let u = serde_json::from_reader(reader)?;
    Ok(u)
}

/// Read a JSON object from file and check the stored version is equal the
/// argument.
pub fn read_exact_versioned_json_from_file<P, T>(version: Version, path: P) -> io::Result<T>
where
    P: AsRef<Path> + std::fmt::Debug,
    T: DeserializeOwned, {
    let versioned: Versioned<T> = read_json_from_file(path)?;
    if versioned.version == version {
        Ok(versioned.value)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Invalid  version in file, expected: {:?}, got: {:?}",
                version, versioned.version,
            ),
        ))
    }
}

/// Read an array of objects from a JSON file and check all versions are equal
/// the argument.
pub fn read_exact_versioned_vec_json_from_file<P, T>(
    version: Version,
    path: P,
) -> io::Result<Vec<T>>
where
    P: AsRef<Path> + std::fmt::Debug,
    T: DeserializeOwned, {
    let versioned: Vec<Versioned<T>> = read_json_from_file(path)?;
    match versioned.iter().find(|v| v.version != version) {
        Some(m) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Invalid version in vectored file, expected: {:?}, got: {:?}",
                version, m.version,
            ),
        )),
        None => Ok(versioned.into_iter().map(|x| x.value).collect()),
    }
}
