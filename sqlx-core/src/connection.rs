use crate::database::{Database, HasStatementCache};
use crate::error::Error;

use crate::transaction::Transaction;
use futures_core::future::BoxFuture;
use log::LevelFilter;
use std::fmt::Debug;
use std::str::FromStr;
use std::time::Duration;
use url::Url;

/// Represents a single database connection.
pub trait Connection: Send {
    type Database: Database;

    type Options: ConnectOptions<Connection = Self>;

    /// Explicitly close this database connection.
    ///
    /// This method is **not required** for safe and consistent operation. However, it is
    /// recommended to call it instead of letting a connection `drop` as the database backend
    /// will be faster at cleaning up resources.
    fn close(self) -> BoxFuture<'static, Result<(), Error>>;

    /// Immediately close the connection without sending a graceful shutdown.
    ///
    /// This should still at least send a TCP `FIN` frame to let the server know we're dying.
    #[doc(hidden)]
    fn close_hard(self) -> BoxFuture<'static, Result<(), Error>>;

    /// Checks if a connection to the database is still valid.
    fn ping(&mut self) -> BoxFuture<'_, Result<(), Error>>;

    /// Begin a new transaction or establish a savepoint within the active transaction.
    ///
    /// Returns a [`Transaction`] for controlling and tracking the new transaction.
    fn begin(&mut self) -> BoxFuture<'_, Result<Transaction<'_, Self::Database>, Error>>
    where
        Self: Sized;

    /// Execute the function inside a transaction.
    ///
    /// If the function returns an error, the transaction will be rolled back. If it does not
    /// return an error, the transaction will be committed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sqlx::postgres::{PgConnection, PgRow};
    /// use sqlx::Connection;
    ///
    /// # pub async fn _f(conn: &mut PgConnection) -> sqlx::Result<Vec<PgRow>> {
    /// conn.transaction(|txn| Box::pin(async move {
    ///     sqlx::query("select * from ..").fetch_all(&mut **txn).await
    /// })).await
    /// # }
    /// ```
    fn transaction<'a, F, R, E>(&'a mut self, callback: F) -> BoxFuture<'a, Result<R, E>>
    where
        for<'c> F: FnOnce(&'c mut Transaction<'_, Self::Database>) -> BoxFuture<'c, Result<R, E>>
            + 'a
            + Send
            + Sync,
        Self: Sized,
        R: Send,
        E: From<Error> + Send,
    {
        Box::pin(async move {
            let mut transaction = self.begin().await?;
            let ret = callback(&mut transaction).await;

            match ret {
                Ok(ret) => {
                    transaction.commit().await?;

                    Ok(ret)
                }
                Err(err) => {
                    transaction.rollback().await?;

                    Err(err)
                }
            }
        })
    }

    /// The number of statements currently cached in the connection.
    fn cached_statements_size(&self) -> usize
    where
        Self::Database: HasStatementCache,
    {
        0
    }

    /// Removes all statements from the cache, closing them on the server if
    /// needed.
    fn clear_cached_statements(&mut self) -> BoxFuture<'_, Result<(), Error>>
    where
        Self::Database: HasStatementCache,
    {
        Box::pin(async move { Ok(()) })
    }

    #[doc(hidden)]
    fn flush(&mut self) -> BoxFuture<'_, Result<(), Error>>;

    #[doc(hidden)]
    fn should_flush(&self) -> bool;

    /// Establish a new database connection.
    ///
    /// A value of [`Options`][Self::Options] is parsed from the provided connection string. This parsing
    /// is database-specific.
    #[inline]
    fn connect(url: &str) -> BoxFuture<'static, Result<Self, Error>>
    where
        Self: Sized,
    {
        let options = url.parse();

        Box::pin(async move { Ok(Self::connect_with(&options?).await?) })
    }

    /// Establish a new database connection with the provided options.
    fn connect_with(options: &Self::Options) -> BoxFuture<'_, Result<Self, Error>>
    where
        Self: Sized,
    {
        options.connect()
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LogSettings {
    pub statements_level: LevelFilter,
    pub slow_statements_level: LevelFilter,
    pub slow_statements_duration: Duration,
}

impl Default for LogSettings {
    fn default() -> Self {
        LogSettings {
            statements_level: LevelFilter::Info,
            slow_statements_level: LevelFilter::Warn,
            slow_statements_duration: Duration::from_secs(1),
        }
    }
}

impl LogSettings {
    pub fn log_statements(&mut self, level: LevelFilter) {
        self.statements_level = level;
    }
    pub fn log_slow_statements(&mut self, level: LevelFilter, duration: Duration) {
        self.slow_statements_level = level;
        self.slow_statements_duration = duration;
    }
}

pub trait ConnectOptions: 'static + Send + Sync + FromStr<Err = Error> + Debug + Clone {
    type Connection: Connection + ?Sized;

    /// Parse the `ConnectOptions` from a URL.
    fn from_url(url: &Url) -> Result<Self, Error>;

    /// Establish a new database connection with the options specified by `self`.
    fn connect(&self) -> BoxFuture<'_, Result<Self::Connection, Error>>
    where
        Self::Connection: Sized;

    /// Log executed statements with the specified `level`
    fn log_statements(self, level: LevelFilter) -> Self;

    /// Log executed statements with a duration above the specified `duration`
    /// at the specified `level`.
    fn log_slow_statements(self, level: LevelFilter, duration: Duration) -> Self;

    /// Entirely disables statement logging (both slow and regular).
    fn disable_statement_logging(self) -> Self {
        self.log_statements(LevelFilter::Off)
            .log_slow_statements(LevelFilter::Off, Duration::default())
    }
}
