use super::{Error, Source, error::MirrorAttempt};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Cursor, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tar::Archive;
use ureq::Agent;

/// User-Agent header to send with download requests.
const USER_AGENT: &str = "https://github.com/drivercraft/ostool";

/// Maximum number of bytes to download (10 MiB).
const MAX_DOWNLOAD_SIZE_IN_BYTES: usize = 10 * 1024 * 1024;
const PROBE_DOWNLOAD_SIZE_IN_BYTES: usize = 256 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const FULL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const FULL_DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FULL_DOWNLOAD_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const FULL_DOWNLOAD_BODY_TIMEOUT: Duration = Duration::from_secs(180);

const OVMF_MIRRORS: &[Mirror] = &[
    Mirror {
        name: "gitee",
        base_url: "https://gitee.com/zr233/ovmf-prebuilt/releases/download",
    },
    Mirror {
        name: "github",
        base_url: "https://github.com/rust-osdev/ovmf-prebuilt/releases/download",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Mirror {
    name: &'static str,
    base_url: &'static str,
}

impl Mirror {
    fn url(self, source: &Source) -> String {
        format!(
            "{base_url}/{release}/{release}-bin.tar.xz",
            base_url = self.base_url,
            release = source.tag
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProbeStats {
    bytes: usize,
    elapsed: Duration,
}

impl ProbeStats {
    fn throughput_score(self) -> u128 {
        (self.bytes as u128) * 1_000_000_000 / self.elapsed.as_nanos().max(1)
    }
}

#[derive(Clone, Debug)]
struct MirrorCandidate {
    mirror: Mirror,
    url: String,
    probe: Option<ProbeStats>,
    index: usize,
}

struct VerifiedDownload {
    actual_hash: String,
    decompressed: Vec<u8>,
}

/// Update the local cache. Does nothing if the cache is already up to date.
pub(crate) fn update_cache(source: Source, prebuilt_dir: &Path) -> Result<(), Error> {
    update_cache_with_fetchers(source, prebuilt_dir, OVMF_MIRRORS, probe_url, download_url)
}

fn update_cache_with_fetchers<P, D>(
    source: Source,
    prebuilt_dir: &Path,
    mirrors: &[Mirror],
    probe: P,
    download: D,
) -> Result<(), Error>
where
    P: FnMut(&str) -> Result<ProbeStats, Error>,
    D: FnMut(&str) -> Result<Vec<u8>, Error>,
{
    let hash_path = prebuilt_dir.join("sha256");

    // Check if the hash file already has the expected hash in it. If so, assume
    // that we've already got the correct prebuilt downloaded and unpacked.
    match fs::read_to_string(&hash_path) {
        Ok(current_hash) if current_hash == source.sha256 => return Ok(()),
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::HashRead {
                path: hash_path.clone(),
                source,
            });
        }
    }

    let candidates = ranked_mirrors_by_probe(&source, mirrors, probe);
    let verified = download_from_candidates(&source, &candidates, download)?;

    // Clear out the existing prebuilt dir, if present.
    if let Err(source) = fs::remove_dir_all(prebuilt_dir)
        && source.kind() != ErrorKind::NotFound
    {
        return Err(Error::RemoveDir {
            path: prebuilt_dir.to_path_buf(),
            source,
        });
    }

    // Extract the files.
    extract(&verified.decompressed, prebuilt_dir)?;

    // Write out the hash file. When we upgrade to a new release of
    // ovmf-prebuilt, the hash will no longer match, triggering a fresh
    // download.
    fs::write(&hash_path, verified.actual_hash).map_err(|source| Error::HashWrite {
        path: hash_path.clone(),
        source,
    })?;

    Ok(())
}

fn ranked_mirrors_by_probe<P>(
    source: &Source,
    mirrors: &[Mirror],
    mut probe: P,
) -> Vec<MirrorCandidate>
where
    P: FnMut(&str) -> Result<ProbeStats, Error>,
{
    let mut candidates = mirrors
        .iter()
        .copied()
        .enumerate()
        .map(|(index, mirror)| {
            let url = mirror.url(source);
            let probe = match probe(&url) {
                Ok(stats) => {
                    info!(
                        "OVMF mirror {} probe: {} bytes in {:?}",
                        mirror.name, stats.bytes, stats.elapsed
                    );
                    Some(stats)
                }
                Err(err) => {
                    debug!(
                        "failed to probe OVMF mirror {} ({}): {}",
                        mirror.name, url, err
                    );
                    None
                }
            };

            MirrorCandidate {
                mirror,
                url,
                probe,
                index,
            }
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left_candidate, right_candidate| {
        match (left_candidate.probe, right_candidate.probe) {
            (Some(left_probe), Some(right_probe)) => right_probe
                .throughput_score()
                .cmp(&left_probe.throughput_score())
                .then_with(|| left_probe.elapsed.cmp(&right_probe.elapsed))
                .then_with(|| left_probe.bytes.cmp(&right_probe.bytes))
                .then_with(|| left_candidate.index.cmp(&right_candidate.index)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left_candidate.index.cmp(&right_candidate.index),
        }
    });

    candidates
}

fn download_from_candidates<D>(
    source: &Source,
    candidates: &[MirrorCandidate],
    mut download: D,
) -> Result<VerifiedDownload, Error>
where
    D: FnMut(&str) -> Result<Vec<u8>, Error>,
{
    let mut attempts = Vec::new();

    for candidate in candidates {
        info!("{}", download_source_message(candidate));
        match download(&candidate.url).and_then(|data| verify_download(source, data)) {
            Ok(verified) => return Ok(verified),
            Err(err) => {
                warn!(
                    "OVMF download from {} failed: {}",
                    candidate.mirror.name, err
                );
                attempts.push(MirrorAttempt {
                    mirror: candidate.mirror.name.to_string(),
                    url: candidate.url.clone(),
                    error: error_with_sources(&err),
                });
            }
        }
    }

    Err(Error::AllMirrorsFailed { attempts })
}

fn download_source_message(candidate: &MirrorCandidate) -> String {
    format!(
        "OVMF download source: {} ({})",
        candidate.mirror.name, candidate.url
    )
}

fn error_with_sources(err: &Error) -> String {
    let mut message = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(err) = source {
        message.push_str(": ");
        message.push_str(&err.to_string());
        source = err.source();
    }
    message
}

fn verify_download(source: &Source, data: Vec<u8>) -> Result<VerifiedDownload, Error> {
    let actual_hash: String = Sha256::digest(&data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual_hash != source.sha256 {
        return Err(Error::HashMismatch {
            actual: actual_hash,
            expected: source.sha256.to_owned(),
        });
    }

    let decompressed = decompress(&data)?;

    Ok(VerifiedDownload {
        actual_hash,
        decompressed,
    })
}

fn http_agent(
    global_timeout: Duration,
    connect_timeout: Duration,
    response_timeout: Duration,
    body_timeout: Duration,
) -> Agent {
    let config = Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_global(Some(global_timeout))
        .timeout_connect(Some(connect_timeout))
        .timeout_recv_response(Some(response_timeout))
        .timeout_recv_body(Some(body_timeout))
        .build();
    Agent::new_with_config(config)
}

fn probe_url(url: &str) -> Result<ProbeStats, Error> {
    let agent = http_agent(PROBE_TIMEOUT, PROBE_TIMEOUT, PROBE_TIMEOUT, PROBE_TIMEOUT);

    info!("probing OVMF mirror {url}");
    let started = Instant::now();
    let resp = agent
        .get(url)
        .header(
            "Range",
            format!("bytes=0-{}", PROBE_DOWNLOAD_SIZE_IN_BYTES - 1),
        )
        .call()
        .map_err(|err| Error::Request(Box::new(err)))?;

    let mut reader = resp
        .into_body()
        .into_reader()
        .take(PROBE_DOWNLOAD_SIZE_IN_BYTES.try_into().unwrap());
    let mut buffer = [0u8; 8192];
    let mut bytes = 0usize;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => bytes += n,
            Err(e) => return Err(Error::Download(e)),
        }
    }

    if bytes == 0 {
        return Err(Error::Download(io::Error::new(
            ErrorKind::UnexpectedEof,
            "mirror probe returned no data",
        )));
    }

    Ok(ProbeStats {
        bytes,
        elapsed: started.elapsed(),
    })
}

/// Download `url` and return the raw data.
fn download_url(url: &str) -> Result<Vec<u8>, Error> {
    let agent = http_agent(
        FULL_DOWNLOAD_TIMEOUT,
        FULL_DOWNLOAD_CONNECT_TIMEOUT,
        FULL_DOWNLOAD_RESPONSE_TIMEOUT,
        FULL_DOWNLOAD_BODY_TIMEOUT,
    );

    // Download the file.
    info!("downloading {url}");
    let resp = agent
        .get(url)
        .call()
        .map_err(|err| Error::Request(Box::new(err)))?;

    // Get content length if available
    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    // Create progress bar
    let progress = if let Some(total) = content_length {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(format!(
            "Downloading {}",
            url.split('/').next_back().unwrap_or("file")
        ));
        pb
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{msg} {spinner:.green} [{elapsed_precise}] {bytes} ({bytes_per_sec})")
                .unwrap(),
        );
        pb.set_message(format!(
            "Downloading {}",
            url.split('/').next_back().unwrap_or("file")
        ));
        pb
    };

    let mut data = Vec::with_capacity(MAX_DOWNLOAD_SIZE_IN_BYTES);
    let mut reader = resp
        .into_body()
        .into_reader()
        // Limit the size of the download.
        .take(MAX_DOWNLOAD_SIZE_IN_BYTES.try_into().unwrap());

    // Read in chunks and update progress
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&buffer[..n]);
                progress.inc(n as u64);
            }
            Err(e) => {
                progress.finish_and_clear();
                return Err(Error::Download(e));
            }
        }
    }

    progress.finish_with_message(format!("Downloaded {} bytes", data.len()));
    info!("received {} bytes", data.len());

    Ok(data)
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, Error> {
    info!("decompressing tarball");
    let mut decompressed = Vec::new();
    let mut compressed = Cursor::new(data);
    lzma_rs::xz_decompress(&mut compressed, &mut decompressed).map_err(Error::Decompress)?;
    Ok(decompressed)
}

/// Extract the tarball's files into `prebuilt_dir`.
///
/// `tarball_data` is raw decompressed tar data.
fn extract(tarball_data: &[u8], prebuilt_dir: &Path) -> Result<(), Error> {
    let cursor = Cursor::new(tarball_data);
    let mut archive = Archive::new(cursor);

    // Extract each file entry.
    for entry in archive.entries().map_err(Error::ArchiveEntries)? {
        let mut entry = entry.map_err(Error::ArchiveEntry)?;

        // Skip directories.
        if entry.size() == 0 {
            continue;
        }

        let path = entry.path().map_err(Error::ArchiveEntryPath)?;
        // Strip the leading directory, which is the release name.
        let path: PathBuf = path.components().skip(1).collect();

        let dir = path.parent().unwrap_or_else(|| Path::new(""));
        let dst_dir = prebuilt_dir.join(dir);
        let dst_path = prebuilt_dir.join(&path);
        info!("unpacking to {}", dst_path.display());
        fs::create_dir_all(&dst_dir).map_err(|source| Error::CreateDir {
            path: dst_dir.clone(),
            source,
        })?;
        entry.unpack(&dst_path).map_err(|source| Error::Unpack {
            path: dst_path.clone(),
            source,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Mirror, MirrorCandidate, ProbeStats, download_source_message, ranked_mirrors_by_probe,
        update_cache_with_fetchers,
    };
    use crate::run::ovmf_prebuilt::{Error, Source};
    use sha2::{Digest, Sha256};
    use std::{
        cell::RefCell,
        fs,
        io::{self, Cursor, ErrorKind},
        time::Duration,
    };
    use tempfile::TempDir;

    const TEST_MIRRORS: &[Mirror] = &[
        Mirror {
            name: "fast",
            base_url: "https://fast.example/releases/download",
        },
        Mirror {
            name: "slow",
            base_url: "https://slow.example/releases/download",
        },
        Mirror {
            name: "backup",
            base_url: "https://backup.example/releases/download",
        },
    ];

    fn sha256_hex(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn test_source(tag: &'static str, sha256: String) -> Source {
        Source {
            tag,
            sha256: Box::leak(sha256.into_boxed_str()),
        }
    }

    fn test_tar_xz(tag: &str, path: &str, contents: &[u8]) -> Vec<u8> {
        let mut tar_data = Vec::new();
        {
            let mut archive = tar::Builder::new(&mut tar_data);
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, format!("{tag}/{path}"), Cursor::new(contents))
                .unwrap();
            archive.finish().unwrap();
        }

        let mut compressed = Vec::new();
        let mut input = Cursor::new(tar_data);
        lzma_rs::xz_compress(&mut input, &mut compressed).unwrap();
        compressed
    }

    fn probe(bytes: usize, millis: u64) -> ProbeStats {
        ProbeStats {
            bytes,
            elapsed: Duration::from_millis(millis),
        }
    }

    #[test]
    fn cache_hit_does_not_probe_or_download() {
        let temp = TempDir::new().unwrap();
        let source = test_source("test-release", "cached-hash".to_string());
        fs::write(temp.path().join("sha256"), source.sha256).unwrap();

        update_cache_with_fetchers(
            source,
            temp.path(),
            TEST_MIRRORS,
            |_| panic!("cache hit should not probe mirrors"),
            |_| panic!("cache hit should not download mirrors"),
        )
        .unwrap();
    }

    #[test]
    fn ranked_mirrors_probe_every_source_and_prefer_fast_successes() {
        let source = test_source("test-release", "unused".to_string());
        let probed = RefCell::new(Vec::new());

        let ranked = ranked_mirrors_by_probe(&source, TEST_MIRRORS, |url| {
            probed.borrow_mut().push(url.to_string());
            if url.contains("fast.example") {
                Ok(probe(256 * 1024, 20))
            } else if url.contains("slow.example") {
                Ok(probe(256 * 1024, 200))
            } else {
                Err(Error::Download(io::Error::new(
                    ErrorKind::TimedOut,
                    "probe timed out",
                )))
            }
        });

        assert_eq!(probed.borrow().len(), TEST_MIRRORS.len());
        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.mirror.name)
                .collect::<Vec<_>>(),
            vec!["fast", "slow", "backup"]
        );
    }

    #[test]
    fn fastest_download_failure_falls_back_to_slower_success() {
        let temp = TempDir::new().unwrap();
        let archive = test_tar_xz("test-release", "x64/code.fd", b"firmware");
        let source = test_source("test-release", sha256_hex(&archive));
        let downloads = RefCell::new(Vec::new());

        update_cache_with_fetchers(
            source,
            temp.path(),
            TEST_MIRRORS,
            |url| {
                if url.contains("fast.example") {
                    Ok(probe(256 * 1024, 10))
                } else if url.contains("slow.example") {
                    Ok(probe(256 * 1024, 100))
                } else {
                    Ok(probe(256 * 1024, 200))
                }
            },
            |url| {
                downloads.borrow_mut().push(url.to_string());
                if url.contains("fast.example") {
                    Err(Error::Download(io::Error::new(
                        ErrorKind::ConnectionReset,
                        "fast mirror reset",
                    )))
                } else if url.contains("slow.example") {
                    Ok(archive.clone())
                } else {
                    panic!("backup mirror should not be downloaded after slow succeeds");
                }
            },
        )
        .unwrap();

        let downloads = downloads.borrow();
        assert_eq!(downloads.len(), 2);
        assert!(downloads[0].contains("fast.example"));
        assert!(downloads[1].contains("slow.example"));
        assert_eq!(
            fs::read_to_string(temp.path().join("sha256")).unwrap(),
            sha256_hex(&archive)
        );
        assert_eq!(
            fs::read(temp.path().join("x64/code.fd")).unwrap(),
            b"firmware"
        );
    }

    #[test]
    fn failed_mirrors_preserve_existing_cache_until_a_valid_download_is_ready() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("x64")).unwrap();
        fs::write(temp.path().join("x64/code.fd"), b"old firmware").unwrap();
        fs::write(temp.path().join("sha256"), "stale-hash").unwrap();

        let archive = test_tar_xz("test-release", "x64/code.fd", b"new firmware");
        let source = test_source("test-release", sha256_hex(&archive));

        let err = update_cache_with_fetchers(
            source,
            temp.path(),
            &TEST_MIRRORS[..2],
            |_| Ok(probe(256 * 1024, 10)),
            |url| {
                if url.contains("fast.example") {
                    Ok(b"wrong hash".to_vec())
                } else {
                    Ok(b"not xz data".to_vec())
                }
            },
        )
        .unwrap_err();

        let Error::AllMirrorsFailed { attempts } = err else {
            panic!("expected aggregate mirror failure");
        };
        assert_eq!(attempts.len(), 2);
        assert!(attempts[0].error.contains("hash"));
        assert!(attempts[1].error.contains("hash") || attempts[1].error.contains("decompress"));
        assert_eq!(
            fs::read(temp.path().join("x64/code.fd")).unwrap(),
            b"old firmware"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("sha256")).unwrap(),
            "stale-hash"
        );
    }

    #[test]
    fn all_download_failures_report_each_mirror() {
        let temp = TempDir::new().unwrap();
        let source = test_source("test-release", "expected-hash".to_string());

        let err = update_cache_with_fetchers(
            source,
            temp.path(),
            &TEST_MIRRORS[..2],
            |_| {
                Err(Error::Download(io::Error::new(
                    ErrorKind::TimedOut,
                    "probe",
                )))
            },
            |url| {
                Err(Error::Download(io::Error::new(
                    ErrorKind::TimedOut,
                    format!("download timed out at {url}"),
                )))
            },
        )
        .unwrap_err();

        let Error::AllMirrorsFailed { attempts } = err else {
            panic!("expected aggregate mirror failure");
        };
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].mirror, "fast");
        assert!(attempts[0].url.contains("fast.example"));
        assert!(attempts[0].error.contains("download timed out"));
        assert_eq!(attempts[1].mirror, "slow");
        assert!(attempts[1].url.contains("slow.example"));
    }

    #[test]
    fn download_source_log_message_names_mirror_and_url() {
        let source = test_source("test-release", "unused".to_string());
        let candidate = MirrorCandidate {
            mirror: TEST_MIRRORS[0],
            url: TEST_MIRRORS[0].url(&source),
            probe: Some(probe(256 * 1024, 10)),
            index: 0,
        };

        let message = download_source_message(&candidate);

        assert_eq!(
            message,
            "OVMF download source: fast (https://fast.example/releases/download/test-release/test-release-bin.tar.xz)"
        );
    }
}
