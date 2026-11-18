//! Fetching release files: the small checksum list and its signature into memory, the large
//! download to a file, hashed as it is written.

use std::io::{Read, Write};

use sha2::Digest as _;

use crate::clock::CancelToken;
use crate::error::{Error, ErrorKind, Result};
use crate::http::HttpClient;

use super::verify::Sha256;

/// More than any release file needs (the downloads are tens of megabytes); a server sending
/// more is not sending a release file.
pub const MAX_DOWNLOAD: u64 = 1 << 30;

/// More than `SHA256SUMS` or its signature needs.
pub const MAX_SMALL: usize = 64 * 1024;

const CHUNK: usize = 64 * 1024;

fn http_error(status: u16, url: &str) -> Error {
    Error::new(ErrorKind::Http)
        .with("status", status)
        .with("url", url)
}

/// GETs a small release file; `None` when the release does not have it (404).
pub fn fetch_small(
    http: &dyn HttpClient,
    url: &str,
    cancel: &CancelToken,
) -> Result<Option<Vec<u8>>> {
    cancel.check()?;
    let response = http.get(url, &[])?;
    cancel.check()?;
    match response.status {
        200..=299 => {}
        404 => return Ok(None),
        status => return Err(http_error(status, url)),
    }
    if response.body.len() > MAX_SMALL {
        return Err(Error::new(ErrorKind::UpdateUnverified)
            .with("url", url)
            .detail("larger than any checksum list or signature"));
    }
    Ok(Some(response.body))
}

/// GETs `url` into `out` and returns the SHA-256 of what was written. `progress` gets the
/// bytes received so far and the total when the server says it. Cancellation is checked
/// between reads. A missing file (404) is [`ErrorKind::UpdateUnavailable`].
pub fn fetch_to(
    http: &dyn HttpClient,
    url: &str,
    out: &mut dyn Write,
    limit: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &CancelToken,
) -> Result<Sha256> {
    cancel.check()?;
    let mut stream = http.get_stream(url, &[])?;
    match stream.status {
        200..=299 => {}
        404 => {
            return Err(Error::new(ErrorKind::UpdateUnavailable)
                .with("url", url)
                .detail("the release has no such file"));
        }
        status => return Err(http_error(status, url)),
    }
    let too_large = || {
        Error::new(ErrorKind::UpdateUnverified)
            .with("url", url)
            .detail(format!("larger than {limit} bytes"))
    };
    if stream.length.is_some_and(|length| length > limit) {
        return Err(too_large());
    }
    let mut hasher = sha2::Sha256::new();
    let mut received: u64 = 0;
    let mut buffer = vec![0u8; CHUNK];
    progress(0, stream.length);
    loop {
        cancel.check()?;
        let read = match stream.body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(Error::new(ErrorKind::Http)
                    .with("url", url)
                    .detail(error.to_string()));
            }
        };
        received += read as u64;
        if received > limit {
            return Err(too_large());
        }
        hasher.update(&buffer[..read]);
        out.write_all(&buffer[..read])?;
        progress(received, stream.length);
    }
    cancel.check()?;
    if let Some(length) = stream.length
        && received != length
    {
        return Err(Error::new(ErrorKind::Http).with("url", url).detail(format!(
            "the download ended after {received} of {length} bytes"
        )));
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::http::{HttpResponse, HttpStream};
    use crate::updater::verify::tests::sha256;

    /// Serves fixed bodies by URL and records what was asked. Streams come in small chunks, so
    /// progress and cancellation between reads are exercised.
    #[derive(Default)]
    pub(crate) struct Files {
        pub(crate) files: HashMap<String, Vec<u8>>,
        /// URLs answered with this status instead.
        pub(crate) statuses: HashMap<String, u16>,
        /// Claim this `Content-Length` instead of the real one.
        pub(crate) lengths: HashMap<String, u64>,
        pub(crate) asked: Mutex<Vec<String>>,
        /// Cancels this token once this many bytes of a stream were read.
        pub(crate) cancel_after: Option<(u64, CancelToken)>,
    }

    impl Files {
        pub(crate) fn with(mut self, url: &str, body: &[u8]) -> Files {
            self.files.insert(url.to_owned(), body.to_vec());
            self
        }

        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    struct Chunks {
        body: std::io::Cursor<Vec<u8>>,
        read: u64,
        cancel_after: Option<(u64, CancelToken)>,
    }

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some((after, token)) = &self.cancel_after
                && self.read >= *after
            {
                token.cancel();
            }
            let n = buf.len().min(1000);
            let read = self.body.read(&mut buf[..n])?;
            self.read += read as u64;
            Ok(read)
        }
    }

    impl HttpClient for Files {
        fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResponse> {
            self.asked.lock().unwrap().push(url.to_owned());
            if let Some(status) = self.statuses.get(url) {
                return Ok(HttpResponse {
                    status: *status,
                    body: Vec::new(),
                });
            }
            Ok(match self.files.get(url) {
                Some(body) => HttpResponse {
                    status: 200,
                    body: body.clone(),
                },
                None => HttpResponse {
                    status: 404,
                    body: Vec::new(),
                },
            })
        }

        fn get_stream(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpStream> {
            let response = self.get(url, headers)?;
            let length = self
                .lengths
                .get(url)
                .copied()
                .unwrap_or(response.body.len() as u64);
            Ok(HttpStream {
                status: response.status,
                length: Some(length),
                body: Box::new(Chunks {
                    body: std::io::Cursor::new(response.body),
                    read: 0,
                    cancel_after: self.cancel_after.clone(),
                }),
            })
        }
    }

    const URL: &str = "https://example.test/file";

    #[test]
    fn a_download_is_written_hashed_and_reported() {
        let body: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let http = Files::default().with(URL, &body);
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let hash = fetch_to(
            &http,
            URL,
            &mut out,
            MAX_DOWNLOAD,
            &mut |received, total| seen.push((received, total)),
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(out, body);
        assert_eq!(hash, sha256(&body));
        assert_eq!(seen.first(), Some(&(0, Some(5000))));
        assert_eq!(seen.last(), Some(&(5000, Some(5000))));
        assert!(seen.len() > 2, "progress after each read: {seen:?}");
    }

    #[test]
    fn a_cancelled_download_stops_between_reads() {
        let token = CancelToken::new();
        let http = Files {
            cancel_after: Some((2000, token.clone())),
            ..Files::default()
        }
        .with(URL, &[7u8; 10_000]);
        let mut out = Vec::new();
        let error =
            fetch_to(&http, URL, &mut out, MAX_DOWNLOAD, &mut |_, _| {}, &token).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert!(out.len() < 10_000);
    }

    #[test]
    fn missing_short_and_oversized_downloads_fail() {
        let mut out = Vec::new();
        let none = Files::default();
        let error = fetch_to(
            &none,
            URL,
            &mut out,
            MAX_DOWNLOAD,
            &mut |_, _| {},
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UpdateUnavailable);

        let mut short = Files::default().with(URL, &[1u8; 100]);
        short.lengths.insert(URL.to_owned(), 200);
        let error = fetch_to(
            &short,
            URL,
            &mut out,
            MAX_DOWNLOAD,
            &mut |_, _| {},
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Http, "ended early");

        let big = Files::default().with(URL, &[1u8; 100]);
        let error =
            fetch_to(&big, URL, &mut out, 99, &mut |_, _| {}, &CancelToken::new()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UpdateUnverified, "over the limit");

        let mut failing = Files::default();
        failing.statuses.insert(URL.to_owned(), 503);
        let error = fetch_to(
            &failing,
            URL,
            &mut out,
            MAX_DOWNLOAD,
            &mut |_, _| {},
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Http);
        assert_eq!(error.param("status"), Some("503"));
    }

    #[test]
    fn small_files_are_fetched_whole_and_a_missing_one_is_none() {
        let http = Files::default().with(URL, b"list");
        assert_eq!(
            fetch_small(&http, URL, &CancelToken::new()).unwrap(),
            Some(b"list".to_vec())
        );
        assert_eq!(
            fetch_small(&http, "https://example.test/other", &CancelToken::new()).unwrap(),
            None
        );
        let big = Files::default().with(URL, &vec![0u8; MAX_SMALL + 1]);
        assert_eq!(
            fetch_small(&big, URL, &CancelToken::new())
                .unwrap_err()
                .kind,
            ErrorKind::UpdateUnverified
        );
    }
}
