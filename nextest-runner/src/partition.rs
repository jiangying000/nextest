// Copyright (c) The nextest Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Support for partitioning test runs across several machines.
//!
//! At the moment this only supports simple hash-based and count-based sharding. In the future it
//! could potentially be made smarter: e.g. using data to pick different sets of binaries and tests
//! to run, with an aim to minimize total build and test times.

use crate::errors::PartitionerBuilderParseError;
use std::{
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};
use twox_hash::XxHash64;

/// A builder for creating `Partitioner` instances.
///
/// The relationship between `PartitionerBuilder` and `Partitioner` is similar to that between
/// `std`'s `BuildHasher` and `Hasher`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PartitionerBuilder {
    /// Partition based on counting test numbers.
    Count {
        /// The shard this is in, counting up from 1.
        shard: u64,

        /// The total number of shards.
        total_shards: u64,
    },

    /// Partition based on hashing. Individual partitions are stateless.
    Hash {
        /// The shard this is in, counting up from 1.
        shard: u64,

        /// The total number of shards.
        total_shards: u64,
    },
}

/// Represents an individual partitioner, typically scoped to a test binary.
pub trait Partitioner: fmt::Debug {
    /// Returns true if the given test name matches the partition.
    fn test_matches(&self, test_name: &str, index: usize) -> bool;
}

impl PartitionerBuilder {
    /// Creates a new `Partitioner` from this `PartitionerBuilder`.
    pub fn build(&self) -> Box<dyn Partitioner> {
        // Note we don't use test_binary at the moment but might in the future.
        match self {
            PartitionerBuilder::Count {
                shard,
                total_shards,
            } => Box::new(CountPartitioner::new(*shard, *total_shards)),
            PartitionerBuilder::Hash {
                shard,
                total_shards,
            } => Box::new(HashPartitioner::new(*shard, *total_shards)),
        }
    }
}

impl FromStr for PartitionerBuilder {
    type Err = PartitionerBuilderParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse the string: it looks like "hash:<shard>/<total_shards>".
        if let Some(input) = s.strip_prefix("hash:") {
            let (shard, total_shards) = parse_shards(input, "hash:M/N")?;

            Ok(PartitionerBuilder::Hash {
                shard,
                total_shards,
            })
        } else if let Some(input) = s.strip_prefix("count:") {
            let (shard, total_shards) = parse_shards(input, "count:M/N")?;

            Ok(PartitionerBuilder::Count {
                shard,
                total_shards,
            })
        } else {
            Err(PartitionerBuilderParseError::new(
                None,
                format!(
                    "partition input '{}' must begin with \"hash:\" or \"count:\"",
                    s
                ),
            ))
        }
    }
}

fn parse_shards(
    input: &str,
    expected_format: &'static str,
) -> Result<(u64, u64), PartitionerBuilderParseError> {
    let mut split = input.splitn(2, '/');
    // First "next" always returns a value.
    let shard_str = split.next().expect("split should have at least 1 element");
    // Second "next" may or may not return a value.
    let total_shards_str = split.next().ok_or_else(|| {
        PartitionerBuilderParseError::new(
            Some(expected_format),
            format!("expected input '{}' to be in the format M/N", input),
        )
    })?;

    let shard: u64 = shard_str.parse().map_err(|err| {
        PartitionerBuilderParseError::new(
            Some(expected_format),
            format!("failed to parse shard '{}' as u64: {}", shard_str, err),
        )
    })?;

    let total_shards: u64 = total_shards_str.parse().map_err(|err| {
        PartitionerBuilderParseError::new(
            Some(expected_format),
            format!(
                "failed to parse total_shards '{}' as u64: {}",
                total_shards_str, err
            ),
        )
    })?;

    // Check that shard > 0 and <= total_shards.
    if !(1..=total_shards).contains(&shard) {
        return Err(PartitionerBuilderParseError::new(
            Some(expected_format),
            format!(
                "shard {} must be a number between 1 and total shards {}, inclusive",
                shard, total_shards
            ),
        ));
    }

    Ok((shard, total_shards))
}

#[derive(Clone, Debug)]
struct CountPartitioner {
    shard_minus_one: u64,
    total_shards: u64,
}

impl CountPartitioner {
    fn new(shard: u64, total_shards: u64) -> Self {
        let shard_minus_one = shard - 1;
        Self {
            shard_minus_one,
            total_shards,
        }
    }
}

impl Partitioner for CountPartitioner {
    fn test_matches(&self, _test_name: &str, index: usize) -> bool {
        (index as u64) % self.total_shards == self.shard_minus_one
    }
}

#[derive(Clone, Debug)]
struct HashPartitioner {
    shard_minus_one: u64,
    total_shards: u64,
}

impl HashPartitioner {
    fn new(shard: u64, total_shards: u64) -> Self {
        let shard_minus_one = shard - 1;
        Self {
            shard_minus_one,
            total_shards,
        }
    }
}

impl Partitioner for HashPartitioner {
    fn test_matches(&self, test_name: &str, _index: usize) -> bool {
        let mut hasher = XxHash64::default();
        test_name.hash(&mut hasher);
        hasher.finish() % self.total_shards == self.shard_minus_one
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitioner_builder_from_str() {
        let successes = vec![
            (
                "hash:1/2",
                PartitionerBuilder::Hash {
                    shard: 1,
                    total_shards: 2,
                },
            ),
            (
                "hash:1/1",
                PartitionerBuilder::Hash {
                    shard: 1,
                    total_shards: 1,
                },
            ),
            (
                "hash:99/200",
                PartitionerBuilder::Hash {
                    shard: 99,
                    total_shards: 200,
                },
            ),
        ];

        let failures = vec![
            "foo",
            "hash",
            "hash:",
            "hash:1",
            "hash:1/",
            "hash:0/2",
            "hash:3/2",
            "hash:m/2",
            "hash:1/n",
            "hash:1/2/3",
        ];

        for (input, output) in successes {
            assert_eq!(
                PartitionerBuilder::from_str(input).unwrap_or_else(|err| panic!(
                    "expected input '{}' to succeed, failed with: {}",
                    input, err
                )),
                output,
                "success case '{}' matches",
                input,
            );
        }

        for input in failures {
            PartitionerBuilder::from_str(input)
                .expect_err(&format!("expected input '{}' to fail", input));
        }
    }
}
