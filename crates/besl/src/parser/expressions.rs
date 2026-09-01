//! Parses BESL declarations and executable syntax from a token stream.

use std::num::{NonZeroU32, NonZeroUsize};

use super::declarations::Precedence;
use super::declarations::{AtomRecordField, Atoms, ExpressionParser, ExpressionParserResult, FeatureParserResult};
use super::declarations::{make_member, make_scope};
use super::iterator::ParserIterator;
use super::*;

mod declarations;
mod dispatch;
mod syntax;

pub(crate) use declarations::*;
pub(crate) use dispatch::*;
pub(crate) use syntax::*;
