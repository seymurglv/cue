//! The speech model's download (spec "Local speech recognition"): every file the folder lacks is fetched as
//! `<name>.part`, resumed where a previous try stopped (an HTTP range request), checked against its size and SHA-256,
//! then renamed. A file under its final name has always passed the check, so a folder with every file at its size is
//! ready. A file that fails the check is deleted: the next try starts it anew.
use std::io::Write;
use std::path::{Path, PathBuf};

use cue_core::model::{Manifest, ModelFile};
use sha2::{Digest, Sha256};

use crate::EngineError;
use crate::error::net;

/// Every file of `m` is in `dir` under its final name and at its size.
pub fn ready(m: &Manifest, dir: &Path) -> bool {
    m.files.iter().all(|f| std::fs::metadata(dir.join(f.name)).is_ok_and(|md| md.len() == f.size))
}

/// Bytes of `m` already in `dir`: finished files and the parts of unfinished ones (what a resumed download skips).
pub fn present(m: &Manifest, dir: &Path) -> u64 {
    m.files
        .iter()
        .map(|f| {
            let size = |p: PathBuf| std::fs::metadata(p).map_or(0, |md| md.len().min(f.size));
            match size(dir.join(f.name)) {
                full if full == f.size => full,
                _ => size(part(dir, f)),
            }
        })
        .sum()
}

fn part(dir: &Path, f: &ModelFile) -> PathBuf {
    dir.join(format!("{}.part", f.name))
}

/// Downloads into `dir` every file of `m` it lacks, from Hugging Face. `progress(done, total)` gets bytes of the whole
/// model as they come.
pub async fn fetch(
    http: &reqwest::Client,
    m: &Manifest,
    dir: &Path,
    progress: impl Fn(u64, u64) + Send,
) -> Result<(), EngineError> {
    fetch_from(http, m, dir, |f| m.url(f), progress).await
}

async fn fetch_from(
    http: &reqwest::Client,
    m: &Manifest,
    dir: &Path,
    url: impl Fn(&ModelFile) -> String,
    progress: impl Fn(u64, u64) + Send,
) -> Result<(), EngineError> {
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Model(format!("model folder: {e}")))?;
    let total = m.total();
    let mut before = 0;
    for f in m.files {
        if !std::fs::metadata(dir.join(f.name)).is_ok_and(|md| md.len() == f.size) {
            let offset = before;
            fetch_file(http, &url(f), f, dir, &|n| progress(offset + n, total)).await?;
        }
        before += f.size;
        progress(before, total);
    }
    Ok(())
}

async fn fetch_file(
    http: &reqwest::Client,
    url: &str,
    f: &ModelFile,
    dir: &Path,
    progress: &impl Fn(u64),
) -> Result<(), EngineError> {
    let part = part(dir, f);
    let have = std::fs::metadata(&part).map_or(0, |md| md.len());
    if have > f.size {
        remove(&part);
        return Err(EngineError::Model(format!("{}: a part larger than the file", f.name)));
    }
    let mut request = http.get(url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request.send().await.map_err(net)?;
    let status = response.status().as_u16();
    // 206: the rest from `have`; 200: the whole file again (the server ignored the range); anything else fails.
    let mut written = match status {
        206 if have > 0 => have,
        200 => 0,
        _ => return Err(EngineError::from_status(status, format!("downloading {}", f.name))),
    };
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(written > 0)
        .write(true)
        .truncate(written == 0)
        .open(&part)
        .map_err(|e| EngineError::Model(format!("{}: {e}", f.name)))?;
    progress(written);
    while let Some(chunk) = response.chunk().await.map_err(net)? {
        written += chunk.len() as u64;
        if written > f.size {
            drop(out);
            remove(&part);
            return Err(EngineError::Model(format!("{}: larger than expected", f.name)));
        }
        out.write_all(&chunk).map_err(|e| EngineError::Model(format!("{}: {e}", f.name)))?;
        progress(written);
    }
    out.sync_all().map_err(|e| EngineError::Model(format!("{}: {e}", f.name)))?;
    drop(out);
    if written < f.size {
        return Err(EngineError::Network(format!("{}: the download stopped at {written} of {} bytes", f.name, f.size)));
    }
    let hash = hash_file(&part).await?;
    if !hash.eq_ignore_ascii_case(f.sha256) {
        remove(&part);
        return Err(EngineError::Model(format!("{}: the file is not the one expected", f.name)));
    }
    std::fs::rename(&part, dir.join(f.name)).map_err(|e| EngineError::Model(format!("{}: {e}", f.name)))
}

/// Every finished file of `m` in `dir` checked against its SHA-256 again, after the model failed to load: a damaged
/// one is deleted, so the next download fetches it anew. Blocking (it reads 670 MB). Returns how many were deleted.
pub fn verify(m: &Manifest, dir: &Path) -> usize {
    let mut removed = 0;
    for f in m.files {
        let path = dir.join(f.name);
        if !path.exists() {
            continue;
        }
        if !hash_blocking(&path).is_ok_and(|h| h.eq_ignore_ascii_case(f.sha256)) {
            remove(&path);
            removed += 1;
        }
    }
    removed
}

fn hash_blocking(path: &Path) -> Result<String, EngineError> {
    let mut file = std::fs::File::open(path).map_err(|e| EngineError::Model(e.to_string()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| EngineError::Model(e.to_string()))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// SHA-256 of a file, on a blocking thread: the encoder is 650 MB.
async fn hash_file(path: &Path) -> Result<String, EngineError> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || hash_blocking(&path)).await.map_err(|e| EngineError::Model(e.to_string()))?
}

fn remove(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!("model: a bad file was not removed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    /// SHA-256 of `BODY`.
    const BODY_SHA: &str = "74e7e5bb9d22d6db26bf76946d40fff3ea9f0346b884fd0694920fccfad15e33";
    const FILES: &[ModelFile] = &[ModelFile { path: "a/one.bin", name: "one.bin", size: 36, sha256: BODY_SHA }];
    const M: Manifest = Manifest { id: "test", name: "Test", repo: "r", commit: "c", files: FILES };

    /// What the test server does: honor ranges, ignore them, or fail.
    #[derive(Clone, Copy, PartialEq)]
    enum Serve {
        Ranges,
        Whole,
        Error,
    }

    /// A one-file HTTP server on localhost; returns its base URL and the Range headers it saw.
    async fn server(serve: Serve, body: &'static [u8]) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let from = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok());
                log.lock().unwrap().push(from.map_or("none".to_owned(), |f| f.to_string()));
                let reply = match (serve, from) {
                    (Serve::Error, _) => b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                    (Serve::Ranges, Some(f)) => {
                        let rest = &body[f..];
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {f}-{}/{}\r\nconnection: close\r\n\r\n",
                            rest.len(),
                            body.len() - 1,
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(rest);
                        r
                    }
                    _ => {
                        let mut r = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                };
                let _ = s.write_all(&reply).await;
                let _ = s.shutdown().await;
            }
        });
        (base, seen)
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-download-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    async fn run(base: &str, dir: &Path) -> (Result<(), EngineError>, Vec<u64>) {
        let seen = Mutex::new(Vec::new());
        let r = fetch_from(&reqwest::Client::new(), &M, dir, |f| format!("{base}/{}", f.path), |d, t| {
            assert_eq!(t, 36);
            seen.lock().unwrap().push(d);
        })
        .await;
        (r, seen.into_inner().unwrap())
    }

    #[tokio::test]
    async fn a_fresh_download_is_checked_and_renamed() {
        let (base, _) = server(Serve::Ranges, BODY).await;
        let dir = folder("fresh");
        let (r, progress) = run(&base, &dir).await;
        r.unwrap();
        assert_eq!(std::fs::read(dir.join("one.bin")).unwrap(), BODY);
        assert!(!part(&dir, &FILES[0]).exists());
        assert!(ready(&M, &dir) && present(&M, &dir) == 36);
        assert_eq!(progress.last(), Some(&36));
        let (r, _) = run("http://127.0.0.1:9", &dir).await;
        assert!(r.is_ok(), "a ready file is not fetched again");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_part_resumes_with_a_range() {
        let (base, seen) = server(Serve::Ranges, BODY).await;
        let dir = folder("resume");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(part(&dir, &FILES[0]), &BODY[..10]).unwrap();
        assert_eq!(present(&M, &dir), 10);
        run(&base, &dir).await.0.unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), ["10"]);
        assert_eq!(std::fs::read(dir.join("one.bin")).unwrap(), BODY);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_server_ignoring_the_range_sends_the_whole_file_again() {
        let (base, _) = server(Serve::Whole, BODY).await;
        let dir = folder("whole");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(part(&dir, &FILES[0]), &BODY[..10]).unwrap();
        run(&base, &dir).await.0.unwrap();
        assert_eq!(std::fs::read(dir.join("one.bin")).unwrap(), BODY);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_wrong_file_is_deleted() {
        let (base, _) = server(Serve::Ranges, b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ").await;
        let dir = folder("wrong");
        let (r, _) = run(&base, &dir).await;
        assert!(matches!(r, Err(EngineError::Model(_))), "{r:?}");
        assert!(!part(&dir, &FILES[0]).exists() && !dir.join("one.bin").exists() && !ready(&M, &dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_server_error_keeps_the_part() {
        let (base, _) = server(Serve::Error, BODY).await;
        let dir = folder("error");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(part(&dir, &FILES[0]), &BODY[..10]).unwrap();
        let (r, _) = run(&base, &dir).await;
        assert!(matches!(r, Err(EngineError::Service(_))), "{r:?}");
        assert_eq!(present(&M, &dir), 10, "the bytes stay for the next try");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verify_deletes_a_damaged_file_only() {
        let dir = folder("verify");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.bin"), BODY).unwrap();
        assert_eq!(verify(&M, &dir), 0);
        let mut bad = BODY.to_vec();
        bad[0] = b'X';
        std::fs::write(dir.join("one.bin"), bad).unwrap();
        assert_eq!(verify(&M, &dir), 1);
        assert!(!dir.join("one.bin").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_short_download_fails_and_keeps_what_came() {
        let (base, _) = server(Serve::Whole, &BODY[..20]).await;
        let dir = folder("short");
        let (r, _) = run(&base, &dir).await;
        assert!(matches!(r, Err(EngineError::Network(_))), "{r:?}");
        assert_eq!(present(&M, &dir), 20);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
