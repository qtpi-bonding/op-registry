//! Cursors used to resume listen streams.

use crate::ListenError;
use op_registry::Cursor;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListenCursor {
    pub epoch: u64,
    pub seq: u64,
}

impl ListenCursor {
    pub fn new(epoch: u64, seq: u64) -> Self {
        Self { epoch, seq }
    }
}

impl fmt::Display for ListenCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.epoch, self.seq)
    }
}

impl FromStr for ListenCursor {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut fields = text.split(':');
        let epoch_text = fields.next();
        let seq_text = fields.next();
        let valid = fields.next().is_none()
            && epoch_text.is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            })
            && seq_text.is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            });

        if !valid {
            return Err(bad_cursor(text));
        }

        let epoch = epoch_text
            .expect("validated cursor epoch")
            .parse::<u64>()
            .map_err(|_| bad_cursor(text))?;
        let seq = seq_text
            .expect("validated cursor sequence")
            .parse::<u64>()
            .map_err(|_| bad_cursor(text))?;
        Ok(Self { epoch, seq })
    }
}

fn bad_cursor(text: &str) -> anyhow::Error {
    coded_error::into_anyhow(ListenError::BadCursor {
        text: text.to_owned(),
    })
}

impl From<ListenCursor> for Cursor {
    fn from(cursor: ListenCursor) -> Self {
        Cursor(cursor.to_string())
    }
}

impl TryFrom<&Cursor> for ListenCursor {
    type Error = anyhow::Error;

    fn try_from(cursor: &Cursor) -> Result<Self, Self::Error> {
        cursor.0.parse()
    }
}
