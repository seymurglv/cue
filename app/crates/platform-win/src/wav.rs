//! Writes the dictation to disk: WAV 16 kHz mono 16-bit (for retry, the batch fallback and the bench).
use std::path::Path;

pub struct WavSink {
    w: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    /// First write error: without it a truncated file would look whole to retry and the batch fallback.
    err: Option<String>,
}

impl WavSink {
    pub fn create(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        Ok(Self { w, err: None })
    }

    /// Stops writing after the first error: the file is already incomplete, `finish` returns the error.
    pub fn write(&mut self, pcm: &[i16]) {
        if self.err.is_some() {
            return;
        }
        for &s in pcm {
            if let Err(e) = self.w.write_sample(s) {
                self.err = Some(e.to_string());
                return;
            }
        }
    }

    /// The header is always written; if writing failed earlier, that first error is returned.
    pub fn finish(self) -> Result<(), String> {
        let done = self.w.finalize().map_err(|e| e.to_string());
        match self.err {
            Some(e) => Err(e),
            None => done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes the test's file or folder even when the check failed.
    struct Remove(std::path::PathBuf);

    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = if self.0.is_dir() { std::fs::remove_dir_all(&self.0) } else { std::fs::remove_file(&self.0) };
        }
    }

    #[test]
    fn writes_16k_mono() {
        let p = std::env::temp_dir().join(format!("cue-wav-{}.wav", std::process::id()));
        let _rm = Remove(p.clone());
        let mut w = WavSink::create(&p).unwrap();
        w.write(&[0i16; 16_000]);
        w.finish().unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 44 + 32_000);
        let r = hound::WavReader::open(&p).unwrap();
        assert_eq!((r.spec().sample_rate, r.spec().channels, r.spec().bits_per_sample), (16_000, 1, 16));
    }

    #[test]
    fn missing_folders_are_created() {
        let dir = std::env::temp_dir().join(format!("cue-wav-dir-{}", std::process::id()));
        let _rm = Remove(dir.clone());
        let p = dir.join("last").join("a.wav");
        let mut w = WavSink::create(&p).unwrap();
        w.write(&[1i16; 320]);
        w.finish().unwrap();
        assert_eq!(hound::WavReader::open(&p).unwrap().len(), 320);
    }

    /// A write failure in the middle of a dictation is not lost: `finish` returns it even if the header was written later.
    /// The failure comes from locking the file through another handle — on Windows the lock is mandatory for everyone.
    #[cfg(windows)]
    #[test]
    fn write_error_is_returned_by_finish() {
        let p = std::env::temp_dir().join(format!("cue-wav-locked-{}.wav", std::process::id()));
        let _rm = Remove(p.clone());
        let mut w = WavSink::create(&p).unwrap();
        let lock = std::fs::File::open(&p).unwrap();
        lock.lock().unwrap();
        w.write(&[0i16; 16_000]); // the 8 KiB buffer overflows — the write to disk hits the lock
        drop(lock);
        assert!(w.finish().is_err(), "without the remembered error the file would look whole but shorter than the dictation");
    }
}
