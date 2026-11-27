// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Locales page (`locales`, plan B141) and the form of one (`locale`): where a guest lives – one language with
//! region, one time zone, one to three keyboards. Same shapes as the Credentials page and form, so every UI draws
//! them as it draws those. An image is built with one locale (the default unless the build chooses another).

use serde::Deserialize;

/// The form's values (kept by the UI while the user chooses)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleValues {
    pub name: Option<String>,
    pub locale: Option<String>,
    pub time_zone: Option<String>,
    pub keyboard: Option<String>,
    pub keyboard2: Option<String>,
    pub keyboard3: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocaleParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: LocaleValues,
}
