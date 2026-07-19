use std::fs::File;
use std::io::{self, BufRead, Read, Seek};
use std::sync::Arc;

pub(super) struct DigestingFile {
    file: File,
    digest: Option<blake3::Hasher>,
    prefix_digest: Option<(blake3::Hasher, u64)>,
    accounting: Arc<super::SourceReadAccounting>,
    remaining_source_bytes: u64,
}

impl DigestingFile {
    pub fn new(
        file: File,
        salt: Option<[u8; 32]>,
        accounting: Arc<super::SourceReadAccounting>,
    ) -> io::Result<Self> {
        Self::with_prefix(file, salt, None, accounting)
    }

    pub fn with_prefix(
        mut file: File,
        salt: Option<[u8; 32]>,
        prefix_bytes: Option<u64>,
        accounting: Arc<super::SourceReadAccounting>,
    ) -> io::Result<Self> {
        let position = file.stream_position()?;
        let remaining_source_bytes = file.metadata()?.len().saturating_sub(position);
        accounting.start_stream(remaining_source_bytes)?;
        let digest = salt.map(|salt| {
            let mut digest = blake3::Hasher::new_keyed(&salt);
            digest.update(b"ccwrapped-source-content/v1\0");
            digest
        });
        let prefix_digest = salt.zip(prefix_bytes).map(|(salt, prefix_bytes)| {
            let mut digest = blake3::Hasher::new_keyed(&salt);
            digest.update(b"ccwrapped-source-content/v1\0");
            (digest, prefix_bytes)
        });
        Ok(Self {
            file,
            digest,
            prefix_digest,
            accounting,
            remaining_source_bytes,
        })
    }

    pub fn finish(self) -> (File, Option<[u8; 32]>, Option<[u8; 32]>) {
        (
            self.file,
            self.digest.map(|digest| *digest.finalize().as_bytes()),
            self.prefix_digest.and_then(|(digest, remaining)| {
                (remaining == 0).then(|| *digest.finalize().as_bytes())
            }),
        )
    }
}

impl Read for DigestingFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining_source_bytes == 0 {
            return Ok(0);
        }
        let maximum = usize::try_from(self.remaining_source_bytes)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let read = self.file.read(&mut buffer[..maximum])?;
        self.remaining_source_bytes = self.remaining_source_bytes.saturating_sub(read as u64);
        self.accounting.record_bytes(read);
        if let Some(digest) = &mut self.digest {
            digest.update(&buffer[..read]);
        }
        if let Some((digest, remaining)) = &mut self.prefix_digest {
            let prefix_read = usize::try_from(*remaining).unwrap_or(usize::MAX).min(read);
            digest.update(&buffer[..prefix_read]);
            *remaining = remaining.saturating_sub(prefix_read as u64);
        }
        Ok(read)
    }
}

#[derive(Debug)]
pub(super) struct BoundedLine {
    pub bytes: Vec<u8>,
    pub oversized: bool,
    pub byte_offset: u64,
}

pub(super) struct BoundedLines<R> {
    reader: R,
    maximum: usize,
    byte_offset: u64,
    finished: bool,
    accounting: Option<Arc<super::SourceReadAccounting>>,
}

impl<R: BufRead> BoundedLines<R> {
    #[allow(dead_code)] // Used by focused reader tests; production paths attach shared accounting.
    pub fn new(reader: R, maximum: usize) -> Self {
        Self {
            reader,
            maximum,
            byte_offset: 0,
            finished: false,
            accounting: None,
        }
    }

    pub fn with_accounting(
        reader: R,
        maximum: usize,
        accounting: Arc<super::SourceReadAccounting>,
    ) -> Self {
        Self {
            reader,
            maximum,
            byte_offset: 0,
            finished: false,
            accounting: Some(accounting),
        }
    }

    pub fn next_line(&mut self) -> io::Result<Option<BoundedLine>> {
        if self.finished {
            return Ok(None);
        }

        let start = self.byte_offset;
        let mut bytes = Vec::new();
        let mut oversized = false;
        loop {
            let buffer = self.reader.fill_buf()?;
            if buffer.is_empty() {
                self.finished = true;
                if bytes.is_empty() && !oversized {
                    return Ok(None);
                }
                break;
            }

            let end = buffer
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(buffer.len(), |position| position.saturating_add(1));
            let chunk = &buffer[..end];
            self.byte_offset = self.byte_offset.saturating_add(chunk.len() as u64);

            if !oversized {
                let remaining = self.maximum.saturating_sub(bytes.len());
                if chunk.len() <= remaining {
                    bytes.extend_from_slice(chunk);
                } else {
                    oversized = true;
                    bytes.clear();
                }
            }

            let found_newline = chunk.last() == Some(&b'\n');
            self.reader.consume(end);
            if found_newline {
                break;
            }
        }

        if bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        if let Some(accounting) = &self.accounting {
            accounting.consume_physical_record()?;
        }
        Ok(Some(BoundedLine {
            bytes,
            oversized,
            byte_offset: start,
        }))
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedLines, DigestingFile};
    use crate::ingestion::SourceReadAccounting;
    use std::fs::{self, File};
    use std::io::{BufReader, Read};
    use std::sync::Arc;

    #[test]
    fn drains_an_oversized_line_and_resumes_at_the_next_record() {
        let input = b"123456789\nok\n";
        let reader = BufReader::with_capacity(3, &input[..]);
        let mut lines = BoundedLines::new(reader, 4);
        let first = lines.next_line().unwrap().unwrap();
        assert!(first.oversized);
        assert!(first.bytes.is_empty());
        let second = lines.next_line().unwrap().unwrap();
        assert!(!second.oversized);
        assert_eq!(second.bytes, b"ok");
        assert_eq!(second.byte_offset, 10);
        assert!(lines.next_line().unwrap().is_none());
    }

    #[test]
    fn rejects_the_physical_record_that_exceeds_the_shared_budget() {
        let input = b"one\ntwo\nthree\n";
        let accounting = Arc::new(SourceReadAccounting::with_limits(u64::MAX, 2));
        let reader = BufReader::new(&input[..]);
        let mut lines = BoundedLines::with_accounting(reader, 16, accounting);

        assert_eq!(lines.next_line().unwrap().unwrap().bytes, b"one");
        assert_eq!(lines.next_line().unwrap().unwrap().bytes, b"two");
        let error = lines.next_line().unwrap_err();
        assert!(error.to_string().contains("E_SOURCE_WORK_LIMIT"));
        assert!(error.to_string().contains("physical-record"));
    }

    #[test]
    fn reserves_each_stream_against_the_invocation_wide_byte_budget() {
        let path = std::env::temp_dir().join(format!(
            "ccwrapped-source-budget-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, b"12345").unwrap();
        let accounting = Arc::new(SourceReadAccounting::with_limits(7, usize::MAX));

        let mut first =
            DigestingFile::new(File::open(&path).unwrap(), None, Arc::clone(&accounting)).unwrap();
        let mut bytes = Vec::new();
        first.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"12345");

        let error = match DigestingFile::new(File::open(&path).unwrap(), None, accounting) {
            Ok(_) => panic!("the second stream must exceed the shared byte budget"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("E_SOURCE_WORK_LIMIT"));
        assert!(error.to_string().contains("source-byte"));
        fs::remove_file(path).unwrap();
    }
}
