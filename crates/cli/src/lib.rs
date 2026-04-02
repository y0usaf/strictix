pub mod config;
pub(crate) mod dirs;
pub mod dump;
pub mod err;
pub mod explain;
pub mod fix;
pub mod lint;
pub mod list;
pub mod traits;

pub mod utils;

use std::collections::HashMap;

use lib::Lint;
use rnix::SyntaxKind;

pub type LintMap = HashMap<SyntaxKind, Vec<&'static dyn Lint>>;
