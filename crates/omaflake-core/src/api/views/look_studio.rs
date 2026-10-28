// SPDX-License-Identifier: AGPL-3.0-or-later
//! The look studio (`lookStudio`, plan B150): a look of your own. A look is data, no Nix – its colours, the terminal's
//! sixteen, the desktop's borders and clock, the editors' themes, macOS guests' system look – so every part of it is a
//! field here: colours with the system's colour picker, the rest as choices. It starts from a look here (one of yours
//! is edited, any other copied) or from omaflake's own; what the studio does not show of it (the desktop's split
//! colours, GTK, the pointer) is kept as it is. A preview shows the colours as a window, a terminal and the desktop's
//! borders; what is hard to read is said while you choose. Saved into My Recipes' folder (`looks/my/<name>/`).
//!
//! Everything chosen is a form value of the page: name, title, description, appearance, tags ("dark, pastel"), base
//! (the look it starts from), plain, `color.<name>`, `normal.<0-7>`, `bright.<0-7>`, `border.<0-1>`,
//! `inactive_border`, `clock`, `prompt.<ok|error|directory>`, neovim, emacs ("<theme>:<variant>"), `macos.<setting>`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::common::*;
use super::text::{tf, Text};
use super::Ctx;
use crate::looks::{Look, LookStore};
use crate::my_recipes::MyRecipes;

pub const PAGE: &str = "lookStudio";

#[derive(Debug, Default, Deserialize)]
pub struct LookStudioParams {
    /// The look it starts from ("my.look.dusk" is edited, any other copied)
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookStudioPage {
    pub title: Text,
    /// The look of yours being edited ("dusk"); None for a new one or a copy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub intro: Text,
    /// Name, title, description, light or dark, tags, the look it starts from, plain
    pub about: Vec<Field>,
    /// The colours and the rest, a section each (none while it is plain)
    pub sections: Vec<LookSection>,
    /// The look as it is now, for a preview a UI draws itself
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<LookPreview>,
    /// What is hard to read (text on its background and the like)
    pub hints: Vec<Hint>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookSection {
    pub id: String,
    pub title: Text,
    /// What the section's colours colour (help)
    pub help: Text,
    pub fields: Vec<Field>,
}

/// The colours of the look now: "#rrggbb" each
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookPreview {
    pub background: String,
    pub surface: String,
    pub selection: String,
    pub text: String,
    pub subtext: String,
    pub muted: String,
    pub accent: String,
    pub urgent: String,
    /// The terminal's eight, then its bright eight
    pub normal: Vec<String>,
    pub bright: Vec<String>,
    /// The active window's border: one colour or a gradient of two
    pub border: Vec<String>,
    pub inactive_border: String,
    pub clock: String,
}

/// The eight colours every look with colours names, with what they colour
const COLORS: [(&str, &str); 8] = [
    ("background", "Background"),
    ("surface", "Surface – panels and bars"),
    ("selection", "Selection"),
    ("text", "Text"),
    ("subtext", "Secondary text"),
    ("muted", "Muted – comments, what is inactive"),
    ("accent", "Accent"),
    ("urgent", "Urgent – errors and warnings"),
];

/// The terminal's colours in their order
const TERMINAL: [&str; 8] = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];

/// The editors' themes from nixpkgs (look schema v1); "" = made from the colours
const NEOVIM: [&str; 9] = [
    "catppuccin:latte",
    "catppuccin:frappe",
    "catppuccin:macchiato",
    "catppuccin:mocha",
    "rose-pine:main",
    "rose-pine:moon",
    "rose-pine:dawn",
    "modus:modus_vivendi",
    "modus:modus_operandi",
];
const EMACS: [&str; 6] = [
    "catppuccin:latte",
    "catppuccin:frappe",
    "catppuccin:macchiato",
    "catppuccin:mocha",
    "modus:modus-vivendi",
    "modus:modus-operandi",
];

/// The key a theme's variant has in look.toml
fn variant_key(theme: &str) -> &'static str {
    match theme {
        "catppuccin" => "flavour",
        "rose-pine" => "variant",
        _ => "style",
    }
}

/// omaflake's own look's colours: a new look starts from them when there is no look here to start from
fn fallback() -> Value {
    json!({
        "appearance": "dark",
        "colors": {"background": "#14161b", "surface": "#1c1f26", "selection": "#2c313c", "text": "#e6e8ee",
            "subtext": "#a9b0bf", "muted": "#6b7385", "accent": "#3fd0e0", "urgent": "#ff5c6c"},
        "terminal": {
            "normal": ["#1c1f26", "#ff5c6c", "#7bd88f", "#fce566", "#5aa9ff", "#c792ea", "#3fd0e0", "#d7dae0"],
            "bright": ["#4b5263", "#ff7a87", "#9be3a9", "#fff08a", "#82bdff", "#d8acf0", "#7fe0ea", "#ffffff"]
        },
        "desktop": {"border": ["#3fd0e0"], "inactive_border": "#2c313c", "clock": "#3fd0e0"}
    })
}

/// "#rrggbb" → (r, g, b) in 0…1
fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some((((v >> 16) & 0xff) as f64 / 255.0, ((v >> 8) & 0xff) as f64 / 255.0, (v & 0xff) as f64 / 255.0))
}

/// WCAG 2's contrast of two colours (1 to 21)
pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let lum = |hex: &str| {
        let (r, g, b) = rgb(hex)?;
        let ch = |c: f64| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        Some(0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b))
    };
    let (x, y) = (lum(a)?, lum(b)?);
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    Some((hi + 0.05) / (lo + 0.05))
}

fn valid_color(v: &Value) -> Option<String> {
    v.as_str().filter(|s| rgb(s).is_some()).map(|s| s.to_lowercase())
}

pub(crate) fn page(ctx: &Ctx, p: &LookStudioParams) -> LookStudioPage {
    let v = &p.values;
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let settings = crate::paths::Settings::load(&ctx.paths);
    let looks = LookStore::new(ctx.paths.clone()).list(&settings);
    let from = p.id.as_ref().and_then(|id| looks.iter().find(|l| &l.id == id));
    let mine = from.filter(|l| l.author == MyRecipes::AUTHOR).map(|l| l.name.clone());

    // What it starts from: the look named, else omaflake's own (or the first with colours), else built-in colours
    let base_ref = text("base").unwrap_or_else(|| from.map(Look::reference).unwrap_or_default());
    let with_colors = |l: &&Look| !l.plain && l.data.get("colors").is_some();
    let base: Option<&Look> = looks.iter().find(|l| l.reference() == base_ref).or(from).or_else(|| {
        looks
            .iter()
            .filter(with_colors)
            .find(|l| l.reference() == "omaflake/omaflake")
            .or(looks.iter().find(with_colors))
    });
    let mut data = base.map(|l| l.data.clone()).unwrap_or_else(fallback);
    if data.get("colors").is_none() {
        // A plain look to start from: the colours come from the built-in ones, should plain be turned off
        let f = fallback();
        for k in ["colors", "terminal", "desktop"] {
            data[k] = f[k].clone();
        }
    }

    // About
    let name = text("name").unwrap_or_else(|| from.map(|l| l.name.clone()).unwrap_or_default());
    let title = text("title").unwrap_or_else(|| from.map(|l| l.title.clone()).unwrap_or_default());
    let description = text("description").unwrap_or_else(|| from.map(|l| l.description.clone()).unwrap_or_default());
    let appearance = text("appearance")
        .or_else(|| data.get("appearance").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "dark".into());
    let tags = text("tags").unwrap_or_else(|| {
        data.get("tags")
            .and_then(Value::as_array)
            .map(|t| t.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    });
    let plain = v.get("plain").and_then(Value::as_bool).unwrap_or_else(|| base.is_some_and(|l| l.plain));
    let taken = looks.iter().any(|l| l.author == MyRecipes::AUTHOR && l.name == name && mine.as_deref() != Some(&name));
    let mut name_field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .value_param("name")
        .help(Text::new("lowercase letters, digits and - – recipes name the look my/<name>"));
    if name.is_empty() || !crate::recipe_sources::RecipeRef::valid_name(&name) {
        name_field.error = Some(Text::new("A name is lowercase letters, digits and -"));
    } else if taken {
        name_field.error = Some(tf!("There is a look \"%@\" of yours already", &name));
    }
    let mut appearance_field =
        Field::new("appearance", FieldKind::Choice, Text::new("Light or dark"), json!(appearance))
            .value_param("appearance");
    appearance_field.choices =
        vec![Choice::new(json!("dark"), Text::new("Dark")), Choice::new(json!("light"), Text::new("Light"))];
    appearance_field.help = Some(Text::new("What programs with a light and a dark mode use – and macOS guests"));
    let mut base_field = Field::new(
        "base",
        FieldKind::Choice,
        Text::new("Starts from"),
        json!(base.map(Look::reference).unwrap_or_default()),
    )
    .value_param("base")
    .help(Text::new("The look whose values it starts with; what you change here wins"));
    base_field.choices = looks
        .iter()
        .map(|l| Choice::new(json!(l.reference()), Text::verbatim(format!("{} – {}", l.title, l.reference()))))
        .collect();
    let mut tags_field = Field::new("tags", FieldKind::Text, Text::new("Tags"), json!(tags)).value_param("tags");
    tags_field.placeholder = Some(Text::new("dark, pastel"));
    let about = vec![
        name_field.clone(),
        Field::new("title", FieldKind::Text, Text::new("Title"), json!(title)).value_param("title"),
        Field::new("description", FieldKind::Text, Text::new("Description"), json!(description))
            .value_param("description"),
        appearance_field,
        tags_field,
        base_field,
        Field::new("plain", FieldKind::Toggle, Text::new("Every program in its own colours"), json!(plain))
            .value_param("plain")
            .help(Text::new("Sets no colours – fonts, layout and keys stay omaflake's (like Vanilla)")),
    ];

    // The values chosen over the base's
    let color = |path: &[&str], key: &str| -> String {
        v.get(key)
            .and_then(valid_color)
            .or_else(|| {
                let mut x = &data;
                for p in path {
                    x = x.get(*p)?;
                }
                valid_color(x)
            })
            .unwrap_or_else(|| "#808080".into())
    };
    let field = |key: &str, label: Text, value: &str| {
        let mut f = Field::new(key, FieldKind::Color, label, json!(value)).value_param(key);
        if let Some(t) = v.get(key).filter(|x| !x.is_null()) {
            if valid_color(t).is_none() {
                f.error = Some(Text::new("A colour is #rrggbb"));
            }
        }
        f
    };
    let colors: Vec<(String, String)> =
        COLORS.iter().map(|(k, _)| (k.to_string(), color(&["colors", k], &format!("color.{k}")))).collect();
    let terminal = |row: &str| -> Vec<String> {
        (0..8)
            .map(|i| {
                let key = format!("{row}.{i}");
                v.get(&key).and_then(valid_color).unwrap_or_else(|| {
                    data["terminal"][row].get(i).and_then(valid_color).unwrap_or_else(|| "#808080".into())
                })
            })
            .collect()
    };
    let (normal, bright) = (terminal("normal"), terminal("bright"));
    let mut border: Vec<String> = (0..2)
        .filter_map(|i| {
            let key = format!("border.{i}");
            match v.get(&key) {
                Some(Value::Null) | None => data["desktop"]["border"].get(i).and_then(valid_color),
                Some(x) if x.as_str() == Some("") => None,
                Some(x) => valid_color(x),
            }
        })
        .collect();
    if border.is_empty() {
        border.push(colors[6].1.clone());
    }
    let inactive = color(&["desktop", "inactive_border"], "inactive_border");
    let clock = color(&["desktop", "clock"], "clock");
    let prompt: Vec<(String, Option<String>)> = ["ok", "error", "directory"]
        .iter()
        .map(|k| {
            let key = format!("prompt.{k}");
            (k.to_string(), v.get(&key).and_then(valid_color).or_else(|| valid_color(&data["desktop"]["prompt"][k])))
        })
        .collect();

    let choice = |key: &str, label: Text, value: &str, options: Vec<(String, Text)>| {
        let mut f = Field::new(key, FieldKind::Choice, label, json!(value)).value_param(key);
        f.choices = options.into_iter().map(|(v, t)| Choice::new(json!(v), t)).collect();
        f
    };
    let theme_of = |editor: &str| -> String {
        text(editor).unwrap_or_else(|| match data.get(editor) {
            Some(t) => {
                let theme = t.get("theme").and_then(Value::as_str).unwrap_or_default();
                let variant = t.get(variant_key(theme)).and_then(Value::as_str).unwrap_or_default();
                if theme.is_empty() {
                    String::new()
                } else {
                    format!("{theme}:{variant}")
                }
            }
            None => String::new(),
        })
    };
    let (neovim, emacs) = (theme_of("neovim"), theme_of("emacs"));
    let themes = |list: &[&str]| {
        let mut out = vec![(String::new(), Text::new("made from the colours"))];
        out.extend(list.iter().map(|t| (t.to_string(), Text::verbatim(t.replace(':', " – ")))));
        out
    };
    let macos_value = |k: &str| {
        text(&format!("macos.{k}"))
            .unwrap_or_else(|| data["macos"].get(k).and_then(Value::as_str).unwrap_or_default().to_string())
    };
    let macos_choice = |k: &str, label: &str, options: &[&str]| {
        let mut list = vec![(String::new(), Text::new("as the look says (light or dark)"))];
        list.extend(options.iter().map(|o| (o.to_string(), Text::verbatim(*o))));
        choice(&format!("macos.{k}"), Text::new(label), &macos_value(k), list)
    };

    let sections = if plain {
        vec![]
    } else {
        vec![
            LookSection {
                id: "colors".into(),
                title: Text::new("Colours"),
                help: Text::new("The eight colours every program of the look is coloured from – bars, menus, the editors, the prompt."),
                fields: COLORS.iter().zip(&colors).map(|((k, label), (_, c))| field(&format!("color.{k}"), Text::new(label), c)).collect(),
            },
            LookSection {
                id: "terminal".into(),
                title: Text::new("Terminal"),
                help: Text::new("The terminal's sixteen colours: eight, and their bright ones – programs in the terminal colour their output with them."),
                fields: TERMINAL
                    .iter()
                    .enumerate()
                    .flat_map(|(i, n)| {
                        [
                            field(&format!("normal.{i}"), Text::new(n), &normal[i]),
                            field(&format!("bright.{i}"), tf!("Bright %@", Text::new(n)), &bright[i]),
                        ]
                    })
                    .collect(),
            },
            LookSection {
                id: "desktop".into(),
                title: Text::new("Desktop"),
                help: Text::new("NixOS's desktop: the active window's border (one colour or a gradient of two), the others' border, the clock, the prompt."),
                fields: {
                    let mut f = vec![
                        field("border.0", Text::new("Active window"), &border[0]),
                        field("border.1", Text::new("Active window, gradient to"), border.get(1).map(String::as_str).unwrap_or(&border[0])),
                        field("inactive_border", Text::new("Other windows"), &inactive),
                        field("clock", Text::new("Clock"), &clock),
                    ];
                    for (k, c) in &prompt {
                        let label = match k.as_str() {
                            "ok" => "Prompt – it worked",
                            "error" => "Prompt – it failed",
                            _ => "Prompt – the folder",
                        };
                        let fallback = match k.as_str() {
                            "ok" => &colors[6].1,
                            "error" => &colors[7].1,
                            _ => &colors[4].1,
                        };
                        f.push(field(&format!("prompt.{k}"), Text::new(label), c.as_deref().unwrap_or(fallback)));
                    }
                    f
                },
            },
            LookSection {
                id: "editors".into(),
                title: Text::new("Editors"),
                help: Text::new("A theme from nixpkgs for Neovim and Emacs, or one made from the colours."),
                fields: vec![
                    choice("neovim", Text::verbatim("Neovim"), &neovim, themes(&NEOVIM)),
                    choice("emacs", Text::verbatim("Emacs"), &emacs, themes(&EMACS)),
                ],
            },
            LookSection {
                id: "macos".into(),
                title: Text::verbatim("macOS"),
                help: Text::new("macOS guests' system look; light or dark comes from above."),
                fields: vec![
                    macos_choice("accent", "Accent colour", &["multicolor", "graphite"]),
                    macos_choice("icons", "Icons", &["default", "clear"]),
                    macos_choice("folders", "Folders", &["default", "graphite"]),
                    macos_choice("wallpaper", "Wallpaper", &["default", "black"]),
                ],
            },
        ]
    };

    // The look as it is now: the base's data with what was chosen here
    let mut look = data.clone();
    if let Value::Object(m) = &mut look {
        for k in ["$schema", "version", "author", "homepage", "description_de"] {
            m.remove(k);
        }
        // look v1 asks for both (a look of yours is MIT like the catalog's)
        m.insert("license".into(), json!("MIT"));
        m.insert("title".into(), json!(if title.is_empty() { name.clone() } else { title.clone() }));
        m.insert("description".into(), json!(description));
        m.insert("appearance".into(), json!(appearance));
        let tag_list: Vec<String> =
            tags.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
        m.insert("tags".into(), json!(tag_list));
        if plain {
            m.insert("plain".into(), json!(true));
            for k in ["colors", "terminal", "desktop", "neovim", "emacs"] {
                m.remove(k);
            }
        } else {
            m.remove("plain");
            m.insert("colors".into(), Value::Object(colors.iter().map(|(k, c)| (k.clone(), json!(c))).collect()));
            m.insert("terminal".into(), json!({"normal": normal, "bright": bright}));
            let mut desktop = data.get("desktop").cloned().unwrap_or_else(|| json!({}));
            desktop["border"] = json!(border);
            desktop["inactive_border"] = json!(inactive);
            desktop["clock"] = json!(clock);
            let chosen: Map<String, Value> =
                prompt.iter().filter_map(|(k, c)| c.as_ref().map(|c| (k.clone(), json!(c)))).collect();
            if !chosen.is_empty() {
                desktop["prompt"] = Value::Object(chosen);
            }
            m.insert("desktop".into(), desktop);
            for (editor, value) in [("neovim", &neovim), ("emacs", &emacs)] {
                match value.split_once(':') {
                    Some((theme, variant)) => {
                        let mut t = json!({"theme": theme});
                        if !variant.is_empty() {
                            t[variant_key(theme)] = json!(variant);
                        }
                        m.insert(editor.into(), t);
                    }
                    None => {
                        m.remove(editor);
                    }
                }
            }
        }
        let macos: Map<String, Value> = ["accent", "icons", "folders", "wallpaper"]
            .iter()
            .filter_map(|k| Some((k.to_string(), json!(Some(macos_value(k)).filter(|x| !x.is_empty())?))))
            .collect();
        if macos.is_empty() {
            m.remove("macos");
        } else {
            m.insert("macos".into(), Value::Object(macos));
        }
    }

    // What is hard to read (WCAG 2: 4.5 for text, 3 for large text and signs)
    let mut hints = Vec::new();
    if !plain {
        let c = |k: &str| colors.iter().find(|(n, _)| n == k).map(|(_, c)| c.as_str()).unwrap_or("#808080");
        let ratio =
            |fg: &str, bg: &str, min: f64| contrast(c(fg), c(bg)).filter(|r| *r < min).map(|r| format!("{r:.1}"));
        if let Some(r) = ratio("text", "background", 4.5) {
            hints.push(Hint::new(
                "warning",
                tf!("Text is hard to read on the background (contrast %@ – at least 4.5)", r),
                Tone::Warning,
            ));
        }
        if let Some(r) = ratio("text", "surface", 4.5) {
            hints.push(Hint::new(
                "warning",
                tf!("Text is hard to read on the surface (contrast %@ – at least 4.5)", r),
                Tone::Warning,
            ));
        }
        if let Some(r) = ratio("subtext", "background", 3.0) {
            hints.push(Hint::new(
                "warning",
                tf!("Secondary text is hard to read (contrast %@ – at least 3)", r),
                Tone::Warning,
            ));
        }
        if let Some(r) = ratio("accent", "background", 3.0) {
            hints.push(Hint::new(
                "warning",
                tf!("The accent hardly stands out from the background (contrast %@ – at least 3)", r),
                Tone::Warning,
            ));
        }
    }

    let preview = (!plain).then(|| {
        let c = |i: usize| colors[i].1.clone();
        LookPreview {
            background: c(0),
            surface: c(1),
            selection: c(2),
            text: c(3),
            subtext: c(4),
            muted: c(5),
            accent: c(6),
            urgent: c(7),
            normal: normal.clone(),
            bright: bright.clone(),
            border: border.clone(),
            inactive_border: inactive.clone(),
            clock: clock.clone(),
        }
    });

    let mut params = json!({"name": name, "look": look});
    if let Some(n) = &mine {
        params["previous"] = json!(n);
    }
    let invalid = sections.iter().flat_map(|s| s.fields.iter()).find_map(|f| f.error.clone());
    let error = name_field.error.clone().or(invalid);
    let save = Action::call("save", Text::new("Save"), Some("apply"), "looks.save", params)
        .then(Then::Back)
        .enabled_if(error.is_none(), error)
        .primary();
    let mut actions = vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({}))];
    if let Some(n) = &mine {
        actions.push(
            Action::call("delete", Text::new("Delete …"), Some("trash"), "looks.delete", json!({"name": n}))
                .destructive()
                .then(Then::Back)
                .confirm(
                    Confirm::new(
                        tf!("Delete the look “%@”?", n),
                        Text::new("Images built with it keep it; new builds choose another."),
                    )
                    .button("Delete")
                    .destructive(),
                ),
        );
    }
    actions.push(save);
    LookStudioPage {
        title: Text::new(if mine.is_some() {
            "Edit Look"
        } else if from.is_some() {
            "Copy Look"
        } else {
            "Look Studio"
        }),
        id: mine,
        intro: Text::new(
            "A look of your own: its colours, the terminal's, the desktop's borders and clock, the editors' themes and macOS guests' system look – chosen here, no Nix needed. The preview shows them; what is hard to read is said while you choose. Saved in My Recipes' folder; recipes and images name the look my/<name>.",
        ),
        about,
        sections,
        preview,
        hints,
        actions,
    }
}
