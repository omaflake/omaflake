// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2) – `Text` and `Arg` live in omaflake-sdk (plan B192); `tf!` builds one here.

pub use omaflake_sdk::text::{Arg, Text};

/// `Text::fmt` with arguments of any type that becomes an `Arg`
macro_rules! tf {
    ($key:expr $(, $arg:expr)* $(,)?) => {
        $crate::api::views::text::Text::fmt($key, vec![$($crate::api::views::text::Arg::from($arg)),*])
    };
}
pub(crate) use tf;
