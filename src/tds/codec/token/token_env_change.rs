use crate::{
    tds::{codec::read_varchar, lcid_to_encoding, sortid_to_encoding},
    Error, SqlReadBytes,
};
use encoding::Encoding;
use fmt::Debug;
use futures::io::AsyncReadExt;
use std::{convert::TryFrom, fmt};

uint_enum! {
    #[repr(u8)]
    pub enum EnvChangeTy {
        Database = 1,
        Language = 2,
        CharacterSet = 3,
        PacketSize = 4,
        UnicodeDataSortingLID = 5,
        UnicodeDataSortingCFL = 6,
        SqlCollation = 7,
        /// below here: >= TDSv7.2
        BeginTransaction = 8,
        CommitTransaction = 9,
        RollbackTransaction = 10,
        EnlistDTCTransaction = 11,
        DefectTransaction = 12,
        RTLS = 13,
        PromoteTransaction = 15,
        TransactionManagerAddress = 16,
        TransactionEnded = 17,
        ResetConnection = 18,
        UserName = 19,
        /// below here: TDS v7.4
        Routing = 20,
    }
}

impl fmt::Display for EnvChangeTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvChangeTy::Database => write!(f, "Database"),
            EnvChangeTy::Language => write!(f, "Language"),
            EnvChangeTy::CharacterSet => write!(f, "CharacterSet"),
            EnvChangeTy::PacketSize => write!(f, "PacketSize"),
            EnvChangeTy::UnicodeDataSortingLID => write!(f, "UnicodeDataSortingLID"),
            EnvChangeTy::UnicodeDataSortingCFL => write!(f, "UnicodeDataSortingCFL"),
            EnvChangeTy::SqlCollation => write!(f, "SqlCollation"),
            EnvChangeTy::BeginTransaction => write!(f, "BeginTransaction"),
            EnvChangeTy::CommitTransaction => write!(f, "CommitTransaction"),
            EnvChangeTy::RollbackTransaction => write!(f, "RollbackTransaction"),
            EnvChangeTy::EnlistDTCTransaction => write!(f, "EnlistDTCTransaction"),
            EnvChangeTy::DefectTransaction => write!(f, "DefectTransaction"),
            EnvChangeTy::RTLS => write!(f, "RTLS"),
            EnvChangeTy::PromoteTransaction => write!(f, "PromoteTransaction"),
            EnvChangeTy::TransactionManagerAddress => write!(f, "TransactionManagerAddress"),
            EnvChangeTy::TransactionEnded => write!(f, "TransactionEnded"),
            EnvChangeTy::ResetConnection => write!(f, "ResetConnection"),
            EnvChangeTy::UserName => write!(f, "UserName"),
            EnvChangeTy::Routing => write!(f, "Routing"),
        }
    }
}

pub struct CollationInfo {
    lcid_encoding: Option<&'static (dyn Encoding + Send + Sync)>,
    sortid_encoding: Option<&'static (dyn Encoding + Send + Sync)>,
}

impl CollationInfo {
    pub fn new(bytes: &[u8]) -> Self {
        let lcid_encoding = match (bytes.get(0), bytes.get(1)) {
            (Some(fst), Some(snd)) => lcid_to_encoding(u16::from_le_bytes([*fst, *snd])),
            _ => None,
        };

        let sortid_encoding = match bytes.get(4) {
            Some(byte) => sortid_to_encoding(*byte),
            _ => None,
        };

        Self {
            lcid_encoding,
            sortid_encoding,
        }
    }
}

impl Debug for CollationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for CollationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.lcid_encoding, self.sortid_encoding) {
            (Some(lcid), Some(sortid)) => write!(f, "{}/{}", lcid.name(), sortid.name()),
            _ => write!(f, "None"),
        }
    }
}

#[derive(Debug)]
pub enum TokenEnvChange {
    Database(String, String),
    PacketSize(u32, u32),
    SqlCollation {
        old: CollationInfo,
        new: CollationInfo,
    },
    BeginTransaction(u64),
    CommitTransaction(u64),
    RollbackTransaction(u64),
    DefectTransaction(u64),
    Routing {
        host: String,
        port: u16,
    },
    ChangeMirror(String),
    Ignored(EnvChangeTy),
}

impl fmt::Display for TokenEnvChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(ref old, ref new) => {
                write!(f, "Database change from '{}' to '{}'", old, new)
            }
            Self::PacketSize(old, new) => {
                write!(f, "Packet size change from '{}' to '{}'", old, new)
            }
            Self::SqlCollation { old, new } => {
                write!(f, "SQL collation change from {} to {}", old, new)
            }
            Self::BeginTransaction(_) => write!(f, "Begin transaction"),
            Self::CommitTransaction(_) => write!(f, "Commit transaction"),
            Self::RollbackTransaction(_) => write!(f, "Rollback transaction"),
            Self::DefectTransaction(_) => write!(f, "Defect transaction"),
            Self::Routing { host, port } => write!(
                f,
                "Server requested routing to a new address: {}:{}",
                host, port
            ),
            Self::ChangeMirror(ref mirror) => write!(f, "Fallback mirror server: `{}`", mirror),
            Self::Ignored(ty) => write!(f, "Ignored env change: `{}`", ty),
        }
    }
}

impl TokenEnvChange {
    pub(crate) async fn decode<R>(src: &mut R) -> crate::Result<Self>
    where
        R: SqlReadBytes + Unpin,
    {
        let _len = src.read_u16_le().await?;
        let ty_byte = src.read_u8().await?;

        let ty = EnvChangeTy::try_from(ty_byte)
            .map_err(|_| Error::Protocol(format!("invalid envchange type {:x}", ty_byte).into()))?;

        let token = match ty {
            EnvChangeTy::Database => {
                let len = src.read_u8().await?;
                let new_value = read_varchar(src, len).await?;

                let len = src.read_u8().await?;
                let old_value = read_varchar(src, len).await?;

                TokenEnvChange::Database(new_value, old_value)
            }
            EnvChangeTy::PacketSize => {
                let len = src.read_u8().await?;
                let new_value = read_varchar(src, len).await?;

                let len = src.read_u8().await?;
                let old_value = read_varchar(src, len).await?;

                TokenEnvChange::PacketSize(new_value.parse()?, old_value.parse()?)
            }
            EnvChangeTy::SqlCollation => {
                let len = src.read_u8().await? as usize;
                let mut new_value = vec![0; len];
                src.read_exact(&mut new_value[0..len]).await?;

                let len = src.read_u8().await? as usize;
                let mut old_value = vec![0; len];
                src.read_exact(&mut old_value[0..len]).await?;

                TokenEnvChange::SqlCollation {
                    new: CollationInfo::new(new_value.as_slice()),
                    old: CollationInfo::new(old_value.as_slice()),
                }
            }
            EnvChangeTy::BeginTransaction | EnvChangeTy::EnlistDTCTransaction => {
                src.read_u8().await?;
                let desc = src.read_u64_le().await?;
                TokenEnvChange::BeginTransaction(desc)
            }
            EnvChangeTy::CommitTransaction => {
                src.read_u8().await?;
                let desc = src.read_u64_le().await?;
                TokenEnvChange::CommitTransaction(desc)
            }
            EnvChangeTy::RollbackTransaction => {
                src.read_u8().await?;
                let desc = src.read_u64_le().await?;
                TokenEnvChange::RollbackTransaction(desc)
            }
            EnvChangeTy::DefectTransaction => {
                src.read_u8().await?;
                let desc = src.read_u64_le().await?;
                TokenEnvChange::DefectTransaction(desc)
            }
            EnvChangeTy::Routing => {
                src.read_u16_le().await?; // routing data value length
                src.read_u8().await?; // routing protocol, always 0 (tcp)

                let port = src.read_u16_le().await?;

                let len = src.read_u16_le().await?; // hostname string length
                let host = read_varchar(src, len).await?;

                // ??? but needed...
                src.read_u16_le().await?;

                TokenEnvChange::Routing { host, port }
            }
            EnvChangeTy::RTLS => {
                let len = src.read_u8().await?;
                let mirror_name = read_varchar(src, len).await?;

                TokenEnvChange::ChangeMirror(mirror_name)
            }
            ty => TokenEnvChange::Ignored(ty),
        };

        Ok(token)
    }
}
