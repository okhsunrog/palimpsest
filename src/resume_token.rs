use std::collections::HashMap;

use thiserror::Error;

use crate::error::ZfsError;
use crate::runner::{Cmd, CommandRunner};

#[derive(Error, Debug)]
pub enum ResumeTokenError {
    #[error("zfs send -nvt failed: {0}")]
    CommandFailed(#[from] ZfsError),

    #[error("missing required field '{field}' in resume token nvlist output")]
    MissingField { field: &'static str },

    #[error("invalid hex value {value:?} for field '{field}'")]
    InvalidHex { field: &'static str, value: String },
}

/// Decoded contents of a ZFS resume token. The token is produced when
/// `zfs recv` is interrupted mid-stream; it encodes enough state for the
/// sender to generate a stream that resumes exactly where the last one stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeToken {
    /// The raw token string (pass to `SendArgs::resume_token()`).
    pub token: String,
    /// Dataset@snapshot name the stream is destined for.
    pub to_name: String,
    /// GUID of the destination snapshot.
    pub to_guid: u64,
    /// GUID of the source snapshot for an incremental resume. `None`
    /// means the partial recv was a full send (no `fromguid` field in
    /// the nvlist). Callers validating "are both endpoints still on
    /// the sender" must skip the from-side check when this is `None`.
    pub from_guid: Option<u64>,
    /// Bytes already received (the resume offset within the stream).
    pub bytes_received: u64,
}

/// `zfs send -nvt <token>` — decode a resume token and return the parsed
/// contents. Also prints the estimated remaining transfer size, which is
/// included in the raw output but not exposed here (use `dry_run()` on the
/// resulting `SendArgs` if you need a fresh estimate).
pub async fn decode(
    runner: &dyn CommandRunner,
    token: &str,
) -> Result<ResumeToken, ResumeTokenError> {
    let output = runner
        .run(Cmd::new("zfs").args(["send", "-nvt", token]))
        .await
        .map_err(ZfsError::Spawn)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResumeTokenError::CommandFailed(
            crate::error::classify_stderr(&stderr, output.status.code()),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_nvlist_output(&text, token)
}

/// Parse the nvlist text block from `zfs send -nvt` output.
///
/// Format:
/// ```text
/// resume token contents:
/// nvlist version: 0
///     object = 0x24
///     offset = 0x0
///     bytes = 0x2a48
///     toguid = 0xd3b96c8266d7cfe6
///     toname = tank/data/home@snap1
/// full send of tank/data/home@snap1 estimated size is 32.5K
/// total estimated size is 32.5K
/// ```
fn parse_nvlist_output(text: &str, token: &str) -> Result<ResumeToken, ResumeTokenError> {
    let mut fields: HashMap<&str, &str> = HashMap::new();

    for line in text.lines() {
        // nvlist fields are tab-indented: "\tkey = value"
        if let Some(kv) = line.strip_prefix('\t')
            && let Some((k, v)) = kv.split_once(" = ")
        {
            fields.insert(k.trim(), v.trim());
        }
    }

    let to_name = fields
        .get("toname")
        .ok_or(ResumeTokenError::MissingField { field: "toname" })?
        .to_string();

    let to_guid = parse_hex_u64(
        fields
            .get("toguid")
            .ok_or(ResumeTokenError::MissingField { field: "toguid" })?,
        "toguid",
    )?;

    let from_guid = match fields.get("fromguid") {
        Some(v) => Some(parse_hex_u64(v, "fromguid")?),
        None => None,
    };

    let bytes_received = parse_hex_u64(
        fields
            .get("bytes")
            .ok_or(ResumeTokenError::MissingField { field: "bytes" })?,
        "bytes",
    )?;

    Ok(ResumeToken {
        token: token.to_string(),
        to_name,
        to_guid,
        from_guid,
        bytes_received,
    })
}

fn parse_hex_u64(s: &str, field: &'static str) -> Result<u64, ResumeTokenError> {
    let hex = s.trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(hex, 16).map_err(|_| ResumeTokenError::InvalidHex {
        field,
        value: s.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RecordingRunner;

    fn load_fixture(name: &str) -> Vec<u8> {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
    }

    #[test]
    fn parse_nvlist_fixture() {
        let raw_token = String::from_utf8(load_fixture("resume_token_raw.txt")).unwrap();
        let token = raw_token.trim();
        let decoded = String::from_utf8(load_fixture("send_resume_token_decoded.txt")).unwrap();
        let result = parse_nvlist_output(&decoded, token).unwrap();
        assert_eq!(result.to_name, "tank/data/home@snap1");
        assert_eq!(result.to_guid, 0xd3b96c8266d7cfe6);
        assert_eq!(result.from_guid, None);
        assert_eq!(result.bytes_received, 0x2a48);
        assert_eq!(result.token, token);
    }

    /// Incremental-resume tokens carry `fromguid` in the nvlist.
    #[test]
    fn parse_nvlist_with_fromguid() {
        let text = "resume token contents:\n\
                    nvlist version: 0\n\
                    \tobject = 0x2\n\
                    \toffset = 0xc0000\n\
                    \tbytes = 0xe1488\n\
                    \ttoguid = 0x9d03e683bc717fa6\n\
                    \tfromguid = 0x123456789abcdef0\n\
                    \ttoname = tank/data@snap2\n";
        let r = parse_nvlist_output(text, "tok").unwrap();
        assert_eq!(r.to_guid, 0x9d03e683bc717fa6);
        assert_eq!(r.from_guid, Some(0x123456789abcdef0));
    }

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex_u64("0x2a48", "bytes").unwrap(), 0x2a48);
        assert_eq!(parse_hex_u64("0x0", "offset").unwrap(), 0);
        assert_eq!(
            parse_hex_u64("0xd3b96c8266d7cfe6", "toguid").unwrap(),
            0xd3b96c8266d7cfe6
        );
    }

    #[test]
    fn parse_hex_invalid() {
        let err = parse_hex_u64("not-hex", "toguid").unwrap_err();
        assert!(matches!(err, ResumeTokenError::InvalidHex { .. }));
    }

    #[test]
    fn parse_missing_field() {
        let err =
            parse_nvlist_output("resume token contents:\nnvlist version: 0\n", "tok").unwrap_err();
        assert!(matches!(
            err,
            ResumeTokenError::MissingField { field: "toname" }
        ));
    }

    #[tokio::test]
    async fn decode_from_fixture() {
        let raw_token = String::from_utf8(load_fixture("resume_token_raw.txt")).unwrap();
        let token = raw_token.trim().to_string();
        let decoded_fixture = load_fixture("send_resume_token_decoded.txt");

        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["send", "-nvt", &token]),
            decoded_fixture,
            vec![],
            0,
        );
        let result = decode(&runner, &token).await.unwrap();
        assert_eq!(result.to_name, "tank/data/home@snap1");
        assert_eq!(result.to_guid, 0xd3b96c8266d7cfe6);
        assert_eq!(result.from_guid, None);
        assert_eq!(result.bytes_received, 0x2a48);
    }
}
