//! The vectors the web app's parsers are tested against too.

use std::collections::BTreeMap;

use serde::Deserialize;
use stemma_core::{Address, references};

#[derive(Deserialize)]
struct Vectors {
    valid: BTreeMap<String, String>,
    invalid: Vec<String>,
    references: Vec<ReferenceCase>,
}

#[derive(Deserialize)]
struct ReferenceCase {
    text: String,
    found: Vec<String>,
}

fn vectors() -> Vectors {
    serde_json::from_str(include_str!("vectors.json")).unwrap()
}

#[test]
fn valid_addresses_parse_to_their_canonical_spelling() {
    for (input, canonical) in vectors().valid {
        let address: Address = input
            .parse()
            .unwrap_or_else(|e| panic!("{input:?} should parse: {e}"));
        assert_eq!(address.to_string(), canonical, "for {input:?}");
    }
}

#[test]
fn invalid_addresses_do_not_parse() {
    for input in vectors().invalid {
        assert!(
            input.parse::<Address>().is_err(),
            "{input:?} should not parse"
        );
    }
}

#[test]
fn references_are_found() {
    for case in vectors().references {
        let found: Vec<&str> = references(&case.text).iter().map(|r| r.raw).collect();
        assert_eq!(found, case.found, "in {:?}", case.text);
    }
}
