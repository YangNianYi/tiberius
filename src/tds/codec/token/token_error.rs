use crate::{
    tds::codec::{read_varchar, FeatureLevel},
    SqlReadBytes, read_u8,
};
use std::fmt;

#[derive(Clone, Debug, thiserror::Error)]
pub struct TokenError {
    /// ErrorCode
    pub(crate) code: u32,
    /// ErrorState (describing code)
    pub(crate) state: u8,
    /// The class (severity) of the error
    pub(crate) class: u8,
    /// The error message
    pub(crate) message: String,
    pub(crate) server: String,
    pub(crate) procedure: String,
    pub(crate) line: u32,
}

impl TokenError {
    pub(crate) async fn decode<R>(src: &mut R) -> crate::Result<Self>
    where
        R: SqlReadBytes + Unpin,
    {
        let _length = src.read_u16_le().await? as usize;
        let code = src.read_u32_le().await?;
        let state = read_u8(src).await?;
        let class = read_u8(src).await?;

        let message_len = src.read_u16_le().await?;
        let message = read_varchar(src, message_len).await?;

        let server_len = read_u8(src).await?;
        let server = read_varchar(src, server_len).await?;

        let procedure_len = read_u8(src).await?;
        let procedure = read_varchar(src, procedure_len).await?;

        let line = if src.context().version > FeatureLevel::SqlServer2005 {
            src.read_u32_le().await?
        } else {
            src.read_u16_le().await? as u32
        };

        let token = TokenError {
            code,
            state,
            class,
            message,
            server,
            procedure,
            line,
        };

        Ok(token)
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "'{}' on server {} executing {} on line {} (code: {}, state: {}, class: {})",
            self.message, self.server, self.procedure, self.line, self.code, self.state, self.class
        )
    }
}