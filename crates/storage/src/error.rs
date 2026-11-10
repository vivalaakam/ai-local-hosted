use std::fmt;

use sea_orm::DbErr;

#[derive(Debug)]
pub enum StorageError {
    NotFound,
    Conflict,
    Duplicate,
    InUse,
    Db(DbErr),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Conflict => f.write_str("a response is already streaming"),
            Self::Duplicate => f.write_str("already exists"),
            Self::InUse => f.write_str("provider is used by conversations"),
            Self::Db(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(err) => Some(err),
            _ => None,
        }
    }
}

impl From<DbErr> for StorageError {
    fn from(err: DbErr) -> Self {
        Self::Db(err)
    }
}
