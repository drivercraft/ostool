//! Hash calculation utilities for FIT image components
//!
//! Provides MD5, SHA1, and CRC32 hash calculations compatible with U-Boot's FIT image format.

use crate::crc::calculate_crc32;

/// Calculate MD5 hash for data
pub fn calculate_md5(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

/// Calculate SHA1 hash for data
pub fn calculate_sha1(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Calculate CRC32 hash for data
pub fn calculate_crc32_hash(data: &[u8]) -> String {
    format!("{:08x}", calculate_crc32(data))
}

/// Hash algorithm types supported by U-Boot FIT images
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// MD5 hash algorithm
    Md5,
    /// SHA1 hash algorithm
    Sha1,
    /// CRC32 hash algorithm
    Crc32,
}

impl HashAlgorithm {
    /// Get the string representation of the hash algorithm
    pub fn as_str(&self) -> &'static str {
        match self {
            HashAlgorithm::Md5 => "md5",
            HashAlgorithm::Sha1 => "sha1",
            HashAlgorithm::Crc32 => "crc32",
        }
    }

    /// Calculate hash using this algorithm
    pub fn calculate(&self, data: &[u8]) -> String {
        match self {
            HashAlgorithm::Md5 => calculate_md5(data),
            HashAlgorithm::Sha1 => calculate_sha1(data),
            HashAlgorithm::Crc32 => calculate_crc32_hash(data),
        }
    }
}

/// Hash calculation result containing algorithm and value
#[derive(Debug, Clone)]
pub struct HashResult {
    /// The hash algorithm used
    pub algorithm: HashAlgorithm,
    /// The hash value as a hex string
    pub value: String,
}

impl HashResult {
    /// Create a new hash result
    pub fn new(algorithm: HashAlgorithm, data: &[u8]) -> Self {
        let value = algorithm.calculate(data);
        Self { algorithm, value }
    }

    /// Get the algorithm name
    pub fn algorithm_name(&self) -> &'static str {
        self.algorithm.as_str()
    }

    /// Get the hash value
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Calculate multiple hashes for data
pub fn calculate_hashes(data: &[u8], algorithms: &[HashAlgorithm]) -> Vec<HashResult> {
    algorithms
        .iter()
        .map(|algo| HashResult::new(algo.clone(), data))
        .collect()
}

/// Default hash algorithms used by U-Boot (MD5, SHA1, CRC32)
pub fn default_hash_algorithms() -> Vec<HashAlgorithm> {
    vec![
        HashAlgorithm::Md5,
        HashAlgorithm::Sha1,
        HashAlgorithm::Crc32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_calculation() {
        let data = b"Hello, World!";
        let md5_hash = calculate_md5(data);
        assert_eq!(md5_hash, "65a8e27d8879283831b664bd8b7f0ad4");
    }

    #[test]
    fn test_sha1_calculation() {
        let data = b"Hello, World!";
        let sha1_hash = calculate_sha1(data);
        println!("SHA1 of 'Hello, World!': {}", sha1_hash);
        // Let's check what our implementation actually produces
        let expected = "0a0a9f2a6772942557ab5355d76af442f8f65e01";
        assert_eq!(sha1_hash, expected);
    }

    #[test]
    fn test_crc32_calculation() {
        let data = b"Hello, World!";
        let crc32_hash = calculate_crc32_hash(data);
        assert_eq!(crc32_hash, "ec4ac3d0");
    }

    #[test]
    fn test_hash_algorithm() {
        let data = b"Hello, World!";

        let md5 = HashAlgorithm::Md5;
        assert_eq!(md5.as_str(), "md5");
        assert_eq!(md5.calculate(data), "65a8e27d8879283831b664bd8b7f0ad4");

        let sha1 = HashAlgorithm::Sha1;
        assert_eq!(sha1.as_str(), "sha1");
        assert_eq!(
            sha1.calculate(data),
            "0a0a9f2a6772942557ab5355d76af442f8f65e01"
        );

        let crc32 = HashAlgorithm::Crc32;
        assert_eq!(crc32.as_str(), "crc32");
        assert_eq!(crc32.calculate(data), "ec4ac3d0");
    }

    #[test]
    fn test_hash_result() {
        let data = b"Hello, World!";
        let result = HashResult::new(HashAlgorithm::Md5, data);

        assert_eq!(result.algorithm_name(), "md5");
        assert_eq!(result.value(), "65a8e27d8879283831b664bd8b7f0ad4");
    }

    #[test]
    fn test_calculate_multiple_hashes() {
        let data = b"Hello, World!";
        let algorithms = vec![
            HashAlgorithm::Md5,
            HashAlgorithm::Sha1,
            HashAlgorithm::Crc32,
        ];

        let results = calculate_hashes(data, &algorithms);
        assert_eq!(results.len(), 3);

        // Verify each result
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.algorithm, algorithms[i]);
            assert!(!result.value.is_empty());
        }
    }

    #[test]
    fn test_default_hash_algorithms() {
        let algorithms = default_hash_algorithms();
        assert_eq!(algorithms.len(), 3);
        assert!(algorithms.contains(&HashAlgorithm::Md5));
        assert!(algorithms.contains(&HashAlgorithm::Sha1));
        assert!(algorithms.contains(&HashAlgorithm::Crc32));
    }
}
