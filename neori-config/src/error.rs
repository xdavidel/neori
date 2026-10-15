use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use miette::Diagnostic;

#[derive(Debug)]
pub struct ConfigParseResult<T, E> {
    pub config: Result<T, E>,

    // We always try to return includes for the file watcher.
    //
    // If the main config is valid, but an included file fails to parse, config will be an Err(),
    // but includes will still be filled, so that fixing just the included file is enough to
    // trigger a reload.
    pub includes: Vec<PathBuf>,
}

/// Error type that chains main errors with include errors.
///
/// Allows miette's Report formatting to have main + include errors all in one.
#[derive(Debug)]
pub struct ConfigIncludeError {
    pub main: String,
    pub includes: Vec<String>,
}

impl<T, E> ConfigParseResult<T, E> {
    pub fn from_err(err: E) -> Self {
        Self {
            config: Err(err),
            includes: Vec::new(),
        }
    }

    pub fn map_config_res<U, V>(
        self,
        f: impl FnOnce(Result<T, E>) -> Result<U, V>,
    ) -> ConfigParseResult<U, V> {
        ConfigParseResult {
            config: f(self.config),
            includes: self.includes,
        }
    }
}

impl fmt::Display for ConfigIncludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.main)
    }
}

impl Error for ConfigIncludeError {}

impl Diagnostic for ConfigIncludeError {
    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        None
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn Diagnostic> + 'a>> {
        None
    }
}
