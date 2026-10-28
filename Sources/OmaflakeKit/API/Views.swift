// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

// The presentation model (docs/api.md §10, plan B121): what each page shows, as the core builds it
// (crates/omaflake-core/src/api/views/). A UI draws these and sends the actions they name. Decoded strictly: a field
// the server always sends is required; fields it leaves out when empty or false (`args`, `ask`, `destructive`, …)
// default to empty/false, as the server's `skip_serializing_if` says. The golden models in
// crates/omaflake-core/testdata/api/views/ pin both sides (APIContractTests).

extension KeyedDecodingContainer {
    /// A field the server leaves out when it is empty or false
    func decode<T: Decodable>(_ key: Key, or fallback: T) throws -> T {
        try decodeIfPresent(T.self, forKey: key) ?? fallback
    }
}

// MARK: - Texts (§10.2)

/// A text a user reads: `key` (the English source string of this app, `%@`/`%lld`), the English `text`, and the
/// values its placeholders take. No key: data shown as it is.
public struct ViewText: Decodable, Sendable, Equatable, Hashable {
    public var key: String?
    public var text: String
    public var args: [TextArg]

    public init(verbatim text: String) {
        key = nil
        self.text = text
        args = []
    }

    public init(key: String, args: [TextArg] = [], text: String? = nil) {
        self.key = key
        self.args = args
        self.text = text ?? key
    }

    enum CodingKeys: String, CodingKey { case key, text, args }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        key = try c.decodeIfPresent(String.self, forKey: .key)
        text = try c.decode(String.self, forKey: .text)
        args = try c.decode(.args, or: [])
    }
}

/// A placeholder's value: a whole number (`%lld`), data, a time, a size, a text of its own, or the value asked for a param
public indirect enum TextArg: Decodable, Sendable, Equatable, Hashable {
    case int(Int64)
    case string(String)
    case date(Date)
    case bytes(Int64)
    case param(String)
    case text(ViewText)

    enum Keys: String, CodingKey { case date, bytes, param }

    public init(from decoder: Decoder) throws {
        let single = try decoder.singleValueContainer()
        if let n = try? single.decode(Int64.self) {
            self = .int(n)
        } else if let s = try? single.decode(String.self) {
            self = .string(s)
        } else {
            let c = try decoder.container(keyedBy: Keys.self)
            if c.contains(.date) {
                self = .date(try c.decode(Date.self, forKey: .date))
            } else if c.contains(.bytes) {
                self = .bytes(try c.decode(Int64.self, forKey: .bytes))
            } else if c.contains(.param) {
                self = .param(try c.decode(String.self, forKey: .param))
            } else {
                self = .text(try ViewText(from: decoder))
            }
        }
    }
}

// MARK: - Building blocks (§10.3)

/// A colour role (design tokens)
public enum Tone: String, OpenEnum { case normal, accent, success, warning, danger, unknown }

public enum LampState: String, OpenEnum { case on, off, attention, working, unknown }

public struct Lamp: Decodable, Sendable, Equatable {
    public var state: LampState
    public var help: ViewText
}

public struct Badge: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var icon: String
    public var help: ViewText
    public var tone: Tone
    public var label: ViewText?
}

public struct InfoLine: Decodable, Sendable, Equatable {
    public var label: ViewText
    public var value: ViewText
    public var note: ViewText?
    public var tone: Tone?
    public var full: String?
}

public struct Hint: Decodable, Sendable, Equatable {
    public var icon: String
    public var text: ViewText
    public var tone: Tone
    public var action: ViewAction?
}

public struct Confirm: Decodable, Sendable, Equatable {
    public struct Input: Decodable, Sendable, Equatable {
        public var param: String
        public var `default`: String
    }

    public var title: ViewText
    public var text: ViewText
    public var button: ViewText
    public var destructive: Bool
    public var input: Input?
}

public enum AskKind: String, OpenEnum { case text, password, folder, file, package, unknown }

/// A value an action asks for before it is sent; it goes into the params at `param` (a dotted path)
public struct Ask: Decodable, Sendable, Equatable {
    public struct Check: Decodable, Sendable, Equatable {
        public var method: String
        public var params: JSONValue
        public var param: String
    }

    public var param: String
    public var kind: AskKind
    public var label: ViewText
    public var prompt: ViewText?
    public var button: ViewText?
    public var `default`: String?
    /// `vm`, `image`, `recipe` (packages), `ipsw` (a file)
    public var accept: String?
    public var multiple: Bool
    public var `repeat`: Bool
    public var check: Check?

    enum CodingKeys: String, CodingKey { case param, kind, label, prompt, button, `default`, accept, multiple, `repeat`, check }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        param = try c.decode(String.self, forKey: .param)
        kind = try c.decode(AskKind.self, forKey: .kind)
        label = try c.decode(ViewText.self, forKey: .label)
        prompt = try c.decodeIfPresent(ViewText.self, forKey: .prompt)
        button = try c.decodeIfPresent(ViewText.self, forKey: .button)
        `default` = try c.decodeIfPresent(String.self, forKey: .default)
        accept = try c.decodeIfPresent(String.self, forKey: .accept)
        multiple = try c.decode(.multiple, or: false)
        `repeat` = try c.decode(.repeat, or: false)
        check = try c.decodeIfPresent(Check.self, forKey: .check)
    }
}

/// What a UI does with a successful answer
public enum Then: String, OpenEnum { case reveal, terminal, back, unknown }

/// Something the user can do: draw `label`/`icon`, grey it out with `reason`, ask `ask` and `confirm`, then send
/// `method` with `params` – a core method or a `ui.*` verb (§10.4)
public struct ViewAction: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var label: ViewText
    /// What a narrow header or toolbar writes instead of `label`
    public var shortLabel: ViewText?
    public var icon: String?
    public var method: String
    public var params: JSONValue
    public var enabled: Bool
    public var reason: ViewText?
    public var help: ViewText?
    public var destructive: Bool
    /// A switch: its state now
    public var on: Bool?
    public var busy: Bool
    public var badge: Int?
    public var ask: [Ask]
    public var confirm: Confirm?
    public var then: Then?
    /// The default action of its row or page (Enter, a double click, a click on a build row), drawn prominent
    public var primary: Bool

    enum CodingKeys: String, CodingKey {
        case id, label, shortLabel, icon, method, params, enabled, reason, help, destructive, on, busy, badge, ask, confirm,
             then, primary
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        label = try c.decode(ViewText.self, forKey: .label)
        shortLabel = try c.decodeIfPresent(ViewText.self, forKey: .shortLabel)
        icon = try c.decodeIfPresent(String.self, forKey: .icon)
        method = try c.decode(String.self, forKey: .method)
        params = try c.decode(JSONValue.self, forKey: .params)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        reason = try c.decodeIfPresent(ViewText.self, forKey: .reason)
        help = try c.decodeIfPresent(ViewText.self, forKey: .help)
        destructive = try c.decode(.destructive, or: false)
        on = try c.decodeIfPresent(Bool.self, forKey: .on)
        busy = try c.decode(.busy, or: false)
        badge = try c.decodeIfPresent(Int.self, forKey: .badge)
        ask = try c.decode(.ask, or: [])
        confirm = try c.decodeIfPresent(Confirm.self, forKey: .confirm)
        then = try c.decodeIfPresent(Then.self, forKey: .then)
        primary = try c.decode(.primary, or: false)
    }
}

extension Array where Element == ViewAction {
    /// The row's or page's default action (`primary`), if it has one
    public var primary: ViewAction? { first(where: \.primary) }
}

/// What the Doctor reports against an action, with the way to it
public struct Blockers: Decodable, Sendable, Equatable {
    public struct Line: Decodable, Sendable, Equatable {
        public var title: ViewText
        public var hint: ViewText
    }

    public var title: ViewText
    public var items: [Line]
    public var action: ViewAction
}

public struct Choice: Decodable, Sendable, Equatable {
    public var value: JSONValue
    public var label: ViewText
    public var icon: String?
}

public struct ViewFilter: Decodable, Sendable, Equatable {
    public enum Kind: String, OpenEnum { case choice, toggle, unknown }

    public struct Group: Decodable, Sendable, Equatable, Identifiable {
        /// The key in `params.filter`
        public var id: String
        public var label: ViewText
        public var kind: Kind
        public var value: JSONValue
        public var options: [Choice]

        enum CodingKeys: String, CodingKey { case id, label, kind, value, options }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            label = try c.decode(ViewText.self, forKey: .label)
            kind = try c.decode(Kind.self, forKey: .kind)
            value = try c.decode(JSONValue.self, forKey: .value)
            options = try c.decode(.options, or: [])
        }
    }

    public var active: Bool
    public var groups: [Group]
}

/// The parts every list page has (flattened into the page's JSON)
public struct ListHead: Decodable, Sendable, Equatable {
    public var title: ViewText
    public var count: Int
    public var of: Int?
    public var filter: ViewFilter?
    public var actions: [ViewAction]
    public var empty: Hint?
}

public enum FieldKind: String, OpenEnum { case toggle, text, number, choice, path, info, unknown }

/// A form field: on a form page the UI keeps the user's value under `params.values.<valueParam>` of the page (and it
/// lands in the page action's params at `param`); in Settings `set` stores it
public struct Field: Decodable, Sendable, Equatable, Identifiable {
    public struct Setter: Decodable, Sendable, Equatable {
        public var method: String
        public var params: JSONValue
        public var param: String
    }

    public var id: String
    public var kind: FieldKind
    public var label: ViewText
    public var value: JSONValue
    public var help: ViewText?
    public var placeholder: ViewText?
    public var choices: [Choice]
    public var min: Int?
    public var max: Int?
    public var step: Int?
    public var enabled: Bool
    public var reason: ViewText?
    public var error: ViewText?
    public var param: String?
    /// The key under the page's `params.values` the user's value is kept under; nil: not kept
    public var valueParam: String?
    public var set: Setter?
    public var ask: Ask?

    enum CodingKeys: String, CodingKey {
        case id, kind, label, value, help, placeholder, choices, min, max, step, enabled, reason, error, param, valueParam,
             set, ask
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decode(FieldKind.self, forKey: .kind)
        label = try c.decode(ViewText.self, forKey: .label)
        value = try c.decode(JSONValue.self, forKey: .value)
        help = try c.decodeIfPresent(ViewText.self, forKey: .help)
        placeholder = try c.decodeIfPresent(ViewText.self, forKey: .placeholder)
        choices = try c.decode(.choices, or: [])
        min = try c.decodeIfPresent(Int.self, forKey: .min)
        max = try c.decodeIfPresent(Int.self, forKey: .max)
        step = try c.decodeIfPresent(Int.self, forKey: .step)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        reason = try c.decodeIfPresent(ViewText.self, forKey: .reason)
        error = try c.decodeIfPresent(ViewText.self, forKey: .error)
        param = try c.decodeIfPresent(String.self, forKey: .param)
        valueParam = try c.decodeIfPresent(String.self, forKey: .valueParam)
        set = try c.decodeIfPresent(Setter.self, forKey: .set)
        ask = try c.decodeIfPresent(Ask.self, forKey: .ask)
    }
}

// MARK: - Pages (§10.5)

public struct HomePage: Decodable, Sendable, Equatable {
    public struct Tab: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var label: ViewText
        public var icon: String
        public var badge: TabBadge?
    }

    /// `count`, `working`, `failed`, `dot`
    public struct TabBadge: Decodable, Sendable, Equatable {
        public var kind: String
        public var count: Int?
        public var tone: Tone
        public var help: ViewText
    }

    /// The window's title ("omaflake", data)
    public var title: ViewText
    public var actions: [ViewAction]
    public var tabs: [Tab]
    public var firstTab: String
}

public struct VmsPage: Decodable, Sendable, Equatable {
    public var head: ListHead
    public var rows: [VmRow]
    public var unreadable: [Hint]

    enum CodingKeys: String, CodingKey { case rows, unreadable }

    public init(from decoder: Decoder) throws {
        head = try ListHead(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        rows = try c.decode([VmRow].self, forKey: .rows)
        unreadable = try c.decode([Hint].self, forKey: .unreadable)
    }
}

public struct VmRow: Decodable, Sendable, Equatable, Identifiable {
    public struct Detail: Decodable, Sendable, Equatable {
        public var switches: [ViewAction]
        public var switchesNote: ViewText?
        public var info: [InfoLine]
        public var actions: [ViewAction]
    }

    public var id: String { name }
    public var name: String
    public var lamp: Lamp
    public var kind: Badge
    public var badges: [Badge]
    public var subtitle: ViewText
    public var actions: [ViewAction]
    public var detail: Detail
    public var vm: Vm
}

public struct VmPage: Decodable, Sendable, Equatable {
    public var title: ViewText
    public var row: VmRow
}

public struct ImagesPage: Decodable, Sendable, Equatable {
    public var head: ListHead
    public var jobs: [BuildJobRow]
    public var rows: [ImageRow]
    public var unreadable: [Hint]
    /// The macOS bases (on a Mac, plan B137)
    public var bases: BasesSection?

    enum CodingKeys: String, CodingKey { case jobs, rows, unreadable, bases }

    public init(from decoder: Decoder) throws {
        head = try ListHead(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        jobs = try c.decode([BuildJobRow].self, forKey: .jobs)
        rows = try c.decode([ImageRow].self, forKey: .rows)
        unreadable = try c.decode([Hint].self, forKey: .unreadable)
        bases = try c.decodeIfPresent(BasesSection.self, forKey: .bases)
    }
}

public struct BuildJobRow: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var name: String
    public var kind: Badge
    public var state: Job.State
    public var title: ViewText
    public var count: ViewText?
    public var fraction: Double?
    public var actions: [ViewAction]
    public var open: ViewAction
    public var job: Job
}

public struct ImageRow: Decodable, Sendable, Equatable, Identifiable {
    public struct Detail: Decodable, Sendable, Equatable {
        public var info: [InfoLine]
        public var actions: [ViewAction]
    }

    public var id: String { name }
    public var name: String
    public var lamp: Lamp
    public var kind: Badge
    public var badges: [Badge]
    public var subtitle: ViewText
    public var usedBy: [String]
    public var actions: [ViewAction]
    public var detail: Detail
    public var image: VmImage
}

public struct ImagePage: Decodable, Sendable, Equatable {
    public var title: ViewText
    public var row: ImageRow
}

public struct RecipesPage: Decodable, Sendable, Equatable {
    public struct Updates: Decodable, Sendable, Equatable {
        public struct Row: Decodable, Sendable, Equatable, Identifiable {
            public var id: String
            public var name: String
            public var kind: Badge
            public var source: String
            public var action: ViewAction
        }

        public var title: ViewText
        public var all: ViewAction?
        public var rows: [Row]
        public var note: ViewText
    }

    public var head: ListHead
    public var catalogsLoaded: Bool
    public var loading: Bool
    /// Drawn next to the spinner while `loading`
    public var loadingText: ViewText?
    public var updates: Updates?
    public var suggestion: Hint?
    public var rows: [RecipeRow]
    /// The heading over `offered`
    public var offeredTitle: ViewText?
    public var offered: [OfferedRow]
    public var unreadable: [Hint]
    public var errors: [Hint]
    public var labHidden: Bool
    /// While `labHidden`: the hint with the way to Settings
    public var labHint: Hint?

    enum CodingKeys: String, CodingKey {
        case catalogsLoaded, loading, loadingText, updates, suggestion, rows, offeredTitle, offered, unreadable, errors,
             labHidden, labHint
    }

    public init(from decoder: Decoder) throws {
        head = try ListHead(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        catalogsLoaded = try c.decode(Bool.self, forKey: .catalogsLoaded)
        loading = try c.decode(Bool.self, forKey: .loading)
        loadingText = try c.decodeIfPresent(ViewText.self, forKey: .loadingText)
        updates = try c.decodeIfPresent(Updates.self, forKey: .updates)
        suggestion = try c.decodeIfPresent(Hint.self, forKey: .suggestion)
        rows = try c.decode([RecipeRow].self, forKey: .rows)
        offeredTitle = try c.decodeIfPresent(ViewText.self, forKey: .offeredTitle)
        offered = try c.decode([OfferedRow].self, forKey: .offered)
        unreadable = try c.decode([Hint].self, forKey: .unreadable)
        errors = try c.decode([Hint].self, forKey: .errors)
        labHidden = try c.decode(Bool.self, forKey: .labHidden)
        labHint = try c.decodeIfPresent(Hint.self, forKey: .labHint)
    }
}

/// The Looks tab (plan B141): the looks here and what the sources offer besides
public struct LooksPage: Decodable, Sendable, Equatable {
    public struct Row: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var name: String
        public var lamp: Lamp
        public var subtitle: ViewText
        public var badges: [Badge]
        /// "#RRGGBB" for a strip of its colours; empty for a plain look; nil on a module's row
        public var swatches: [String]?
        public var actions: [ViewAction]
        public var detail: ImageRow.Detail
    }

    public var head: ListHead
    public var catalogsLoaded: Bool
    public var loading: Bool
    public var loadingText: ViewText?
    public var rows: [Row]
    public var offeredTitle: ViewText?
    public var offered: [Row]
    public var unreadable: [Hint]
    public var footer: ViewText

    enum CodingKeys: String, CodingKey {
        case catalogsLoaded, loading, loadingText, rows, offeredTitle, offered, unreadable, footer
    }

    public init(from decoder: Decoder) throws {
        head = try ListHead(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        catalogsLoaded = try c.decode(Bool.self, forKey: .catalogsLoaded)
        loading = try c.decode(Bool.self, forKey: .loading)
        loadingText = try c.decodeIfPresent(ViewText.self, forKey: .loadingText)
        rows = try c.decode([Row].self, forKey: .rows)
        offeredTitle = try c.decodeIfPresent(ViewText.self, forKey: .offeredTitle)
        offered = try c.decode([Row].self, forKey: .offered)
        unreadable = try c.decode([Hint].self, forKey: .unreadable)
        footer = try c.decode(ViewText.self, forKey: .footer)
    }
}

/// `keys` (plan B147): a recipe's keys – of an image or a VM through its recipe – by module and mode
public struct KeysPage: Decodable, Sendable, Equatable {
    public struct Module: Decodable, Sendable, Equatable, Identifiable {
        public var reference: String
        public var title: String
        public var groups: [Group]
        public var id: String { reference }
    }

    /// A mode's keys (or those without a mode) with the way there from the start mode
    public struct Group: Decodable, Sendable, Equatable {
        public var mode: String?
        public var title: ViewText
        public var start: Bool
        public var way: [Step]
        public var rows: [Row]
    }

    public struct Step: Decodable, Sendable, Equatable {
        /// The key's chords, each a list of caps
        public var caps: [[String]]
        public var mode: ViewText
    }

    public struct Row: Decodable, Sendable, Equatable {
        public var action: String
        public var keys: [String]
        /// Each key's chords, each a list of caps
        public var caps: [[[String]]]
        public var title: ViewText
        public var after: ViewText?
        public var backToStart: Bool
        public var switches: Bool
        public var does: String?
        public var from: String?
        public var own: Bool
    }

    public var title: ViewText
    public var subtitle: ViewText
    public var intro: ViewText
    public var notes: [Hint]
    public var modules: [Module]
    public var keymaps: [String]
    public var search: ViewText
}

public struct RecipeRow: Decodable, Sendable, Equatable, Identifiable {
    public struct Detail: Decodable, Sendable, Equatable {
        public var description: String?
        public var info: [InfoLine]
        /// What is missing or wrong (plan B142); its keys are on the keys page (plan B147)
        public var notes: [Hint]?
        public var versions: [VersionRow]
        public var actions: [ViewAction]
    }

    public var id: String
    public var line: String
    public var name: String
    public var lamp: Lamp
    public var kind: Badge
    /// Its author when not omaflake, its distinction
    public var secondary: [String]
    public var badges: [Badge]
    public var tags: [String]
    public var actions: [ViewAction]
    public var detail: Detail
    public var recipe: Recipe
}

public struct VersionRow: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var words: String
    public var state: ViewText
    public var actions: [ViewAction]
    public var version: Recipe.Version
}

public struct OfferedRow: Decodable, Sendable, Equatable, Identifiable {
    public struct Detail: Decodable, Sendable, Equatable {
        public var description: String
        public var info: [InfoLine]
        public var warning: ViewText?
    }

    public var id: String
    public var name: String
    public var author: String
    public var lamp: Lamp
    public var kind: Badge
    public var badges: [Badge]
    public var tags: [String]
    public var actions: [ViewAction]
    public var detail: Detail
    public var recipe: CatalogRecipe
}

public struct FormSection: Decodable, Sendable, Equatable {
    public var title: ViewText?
    public var fields: [Field]
}

public struct NewVmPage: Decodable, Sendable, Equatable {
    public var title: ViewText
    public var image: String
    public var recipe: String
    public var kind: Badge
    public var subtitle: ViewText
    public var description: String?
    public var lab: Bool
    public var blockers: Blockers?
    public var sections: [FormSection]
    public var warnings: [ViewText]
    public var valid: Bool
    public var actions: [ViewAction]
}

public enum BuildState: String, OpenEnum { case idle, queued, checking, running, finished, failed, cancelled, unknown }

public struct BuildPage: Decodable, Sendable, Equatable {
    public struct Progress: Decodable, Sendable, Equatable {
        public var step: ViewText
        public var title: ViewText
        public var count: ViewText?
        public var fraction: Double
        public var started: Date?
        public var lastOutput: Date?
        /// Seconds without output after which the quiet time turns orange
        public var quietWarning: Int
        public var quietHelp: ViewText
    }

    public struct Outcome: Decodable, Sendable, Equatable {
        public var state: BuildState
        public var icon: String
        public var tone: Tone
        public var title: ViewText
        public var detail: ViewText?
        public var action: ViewAction?
    }

    public var title: ViewText
    public var recipe: String
    public var name: String
    public var kind: Badge
    public var badges: [Badge]
    public var description: String?
    public var state: BuildState
    public var actions: [ViewAction]
    public var blockers: Blockers?
    public var macBase: Hint?
    /// What the recipe is made of and what is wrong (plan B142)
    public var notes: [Hint]?
    public var status: ViewText?
    public var progress: Progress?
    public var outcome: Outcome?
    public var newest: ViewText?
    public var log: LogRef?
    public var job: Job?
    /// The credential and look of the image (plan B137); nil while a build runs
    public var fields: [Field]?
}

/// The Credentials page (plan B137): who you are in a guest, one row per credential, "New Credential …"
public struct CredentialsPage: Decodable, Sendable, Equatable {
    public struct Row: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var name: String
        public var lamp: Lamp
        public var subtitle: ViewText
        public var badges: [Badge]
        public var usedBy: [String]
        /// The quick action: Copy Password
        public var actions: [ViewAction]
        public var detail: ImageRow.Detail
    }

    public var title: ViewText
    public var intro: ViewText
    public var rows: [Row]
    public var add: ViewAction
    public var footer: ViewText
}

/// One credential's form (plan B137): new, or to edit – only the name while images use it
public struct CredentialPage: Decodable, Sendable, Equatable {
    public var title: ViewText
    public var id: String?
    public var intro: ViewText
    public var fields: [Field]
    public var locked: ViewText?
    public var actions: [ViewAction]
}

/// The macOS bases on the images page (plan B137)
public struct BasesSection: Decodable, Sendable, Equatable {
    public struct Row: Decodable, Sendable, Equatable, Identifiable {
        public var id: String { name }
        public var name: String
        public var lamp: Lamp
        public var subtitle: ViewText
        public var badges: [Badge]
        public var actions: [ViewAction]
    }

    public var title: ViewText
    public var rows: [Row]
    public var empty: Hint?
    /// The base being created (IPSW download, then install): its progress; a click opens its page
    public var job: BuildJobRow?
    /// Shown only without `empty` – the hint has it then
    public var create: ViewAction
    public var footer: ViewText
}

/// Where a job's log is: `jobs.log {id: job, from}` reads what is kept, then `job.log` brings new lines
public struct LogRef: Decodable, Sendable, Equatable {
    public var job: String
    /// The first line `jobs.log` keeps
    public var from: Int
    public var lines: Int
    public var note: ViewText?
    public var file: String?
    public var full: ViewAction?
}

public struct Download: Decodable, Sendable, Equatable {
    public var done: Int64
    public var of: Int64
    public var text: ViewText
}

public struct DoctorPage: Decodable, Sendable, Equatable {
    public struct UpdateLine: Decodable, Sendable, Equatable {
        public var label: ViewText
        public var text: ViewText
        public var tone: Tone
        public var busy: Bool
        public var actions: [ViewAction]
    }

    public struct Section: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var title: ViewText
        public var items: [Item]
    }

    public struct Item: Decodable, Sendable, Equatable, Identifiable {
        public var id: String { item.rawValue }
        public var item: Prerequisite.Item
        public var status: Prerequisite.Status
        public var title: ViewText
        public var detail: ViewText?
        public var hint: ViewText?
        public var expandable: Bool
        public var expanded: Bool
        public var busy: Bool
        public var task: ViewText?
        public var download: Download?
        public var actions: [ViewAction]
        public var prerequisite: Prerequisite
    }

    public var title: ViewText
    public var versions: [InfoLine]
    public var update: UpdateLine
    public var intro: ViewText
    public var checking: Bool
    public var checked: Date?
    public var actions: [ViewAction]
    public var sections: [Section]
    public var issues: Int
}

public struct MacBasePage: Decodable, Sendable, Equatable {
    public struct IpswLine: Decodable, Sendable, Equatable {
        public var label: ViewText
        public var text: ViewText
        public var download: Download?
        public var actions: [ViewAction]
    }

    public var title: ViewText
    public var intro: ViewText
    public var base: Hint?
    public var ipsw: IpswLine
    public var fields: [Field]
    public var installing: Bool
    public var actions: [ViewAction]
    /// The latest base creation's job log
    public var log: LogRef?
}

public struct SettingsPage: Decodable, Sendable, Equatable {
    public struct Section: Decodable, Sendable, Equatable {
        public var title: ViewText
        public var fields: [Field]
        public var footer: ViewText?
    }

    public struct Sources: Decodable, Sendable, Equatable {
        public struct Row: Decodable, Sendable, Equatable, Identifiable {
            public var id: String
            public var name: String
            public var icon: String
            public var badges: [Badge]
            public var subtitle: ViewText
            public var actions: [ViewAction]
            public var source: Source
        }

        public var title: ViewText
        public var rows: [Row]
        public var add: Section
        public var addAction: ViewAction
        public var footer: ViewText
    }

    public struct DataFolder: Decodable, Sendable, Equatable {
        public var title: ViewText
        public var location: String
        public var isDefault: Bool
        public var moving: Bool
        public var actions: [ViewAction]
        public var note: ViewText
    }

    public var title: ViewText
    public var sources: Sources
    public var general: Section
    public var guest: Section
    public var dataFolder: DataFolder
}

public struct ActivityPage: Decodable, Sendable, Equatable {
    public struct Entry: Decodable, Sendable, Equatable, Identifiable {
        public var id: Int
        public var time: Date
        public var level: ServerEvent.Level
        public var text: ViewText
        public var notification: Notification?
    }

    /// What a system notification says
    public struct Notification: Decodable, Sendable, Equatable {
        public var title: ViewText
        public var body: ViewText
    }

    public var head: ListHead
    public var entries: [Entry]
    /// Notifications are on (Settings)
    public var notifications: Bool

    enum CodingKeys: String, CodingKey { case entries, notifications }

    public init(from decoder: Decoder) throws {
        head = try ListHead(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        entries = try c.decode([Entry].self, forKey: .entries)
        notifications = try c.decode(Bool.self, forKey: .notifications)
    }
}

/// The import step (§10.4): run `action` (asking its `confirm`) or show `reason`
public struct ImportPage: Decodable, Sendable, Equatable {
    /// The dialog's title
    public var title: ViewText
    public var path: String
    public var name: String
    public var kind: PackageInspection.Kind
    public var valid: Bool
    public var reason: ViewText?
    public var action: ViewAction?
    public var inspection: PackageInspection
}

// MARK: - Getting and following pages (§10.1)

/// A page and its params, as `views.get` and `views.subscribe` take them
public struct PageRef: Codable, Sendable, Hashable {
    public var page: String
    public var params: JSONValue?

    public init(_ page: String, _ params: JSONValue? = .object([:])) {
        self.page = page
        self.params = params
    }

    enum CodingKeys: String, CodingKey { case page, params }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(page, forKey: .page)
        try c.encodeIfPresent(params, forKey: .params)
    }

    /// The params as the server echoes them (`{}` when none)
    public var echoed: JSONValue { params ?? .object([:]) }
}

public struct ViewError: Decodable, Sendable, Equatable {
    public var message: String
    public var kind: String?
    /// The message as a text when it has a key (api.md §3)
    public var text: ViewText?
}

/// A page's model, by the page it is
public enum PageModel: Sendable, Equatable {
    case home(HomePage), vms(VmsPage), vm(VmPage), images(ImagesPage), image(ImagePage), recipes(RecipesPage)
    case newVm(NewVmPage), build(BuildPage), doctor(DoctorPage), macBase(MacBasePage), settings(SettingsPage)
    case activity(ActivityPage), importStep(ImportPage), credentials(CredentialsPage), credential(CredentialPage)
    case looks(LooksPage), keys(KeysPage)
    /// The page could not be built
    case error(ViewError)
    /// A page this client does not know
    case unknown(String)

    enum ErrorKey: String, CodingKey { case error }

    public init(page: String, from decoder: Decoder) throws {
        if let c = try? decoder.container(keyedBy: ErrorKey.self), c.contains(.error) {
            self = .error(try c.decode(ViewError.self, forKey: .error))
            return
        }
        switch page {
        case "home": self = .home(try HomePage(from: decoder))
        case "vms": self = .vms(try VmsPage(from: decoder))
        case "vm": self = .vm(try VmPage(from: decoder))
        case "images": self = .images(try ImagesPage(from: decoder))
        case "image": self = .image(try ImagePage(from: decoder))
        case "recipes": self = .recipes(try RecipesPage(from: decoder))
        case "newVm": self = .newVm(try NewVmPage(from: decoder))
        case "build": self = .build(try BuildPage(from: decoder))
        case "doctor": self = .doctor(try DoctorPage(from: decoder))
        case "macBase": self = .macBase(try MacBasePage(from: decoder))
        case "settings": self = .settings(try SettingsPage(from: decoder))
        case "credentials": self = .credentials(try CredentialsPage(from: decoder))
        case "credential": self = .credential(try CredentialPage(from: decoder))
        // Locales (plan B141): the same page and form as credentials
        case "locales": self = .credentials(try CredentialsPage(from: decoder))
        case "locale": self = .credential(try CredentialPage(from: decoder))
        // Plan B142: profiles as credentials, the recipe editor as a credential's form, modules as looks
        case "profiles": self = .credentials(try CredentialsPage(from: decoder))
        case "profile", "recipeEditor": self = .credential(try CredentialPage(from: decoder))
        case "modules": self = .looks(try LooksPage(from: decoder))
        // The credential of a macOS base from before bases kept theirs: the same form (plan B139)
        case "baseCredential": self = .credential(try CredentialPage(from: decoder))
        case "activity": self = .activity(try ActivityPage(from: decoder))
        case "looks": self = .looks(try LooksPage(from: decoder))
        case "import": self = .importStep(try ImportPage(from: decoder))
        case "keys": self = .keys(try KeysPage(from: decoder))
        default: self = .unknown(page)
        }
    }

    /// A page's model from its JSON
    public static func decode(page: String, from data: Data) throws -> PageModel {
        let decoder = JSONDecoder.api
        decoder.userInfo[.viewPage] = page
        return try decoder.decode(Box.self, from: data).model
    }

    struct Box: Decodable {
        let model: PageModel
        init(from decoder: Decoder) throws {
            model = try PageModel(page: decoder.userInfo[.viewPage] as? String ?? "", from: decoder)
        }
    }
}

extension CodingUserInfoKey {
    static let viewPage = CodingUserInfoKey(rawValue: "app.omaflake.viewPage")!
}

/// `view.changed` params
struct ViewChanged: Decodable {
    let page: String
    let params: JSONValue
    let model: PageModel

    enum CodingKeys: String, CodingKey { case page, params, model }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        page = try c.decode(String.self, forKey: .page)
        params = try c.decode(JSONValue.self, forKey: .params)
        model = try PageModel(page: page, from: c.superDecoder(forKey: .model))
    }
}

// MARK: - Params

extension JSONValue {
    /// Sets `value` at the dotted `path` (an object; objects are created on the way) – where an asked value or a
    /// form field goes in an action's params (§10.3)
    public mutating func set(_ value: JSONValue, at path: String) {
        let parts = path.split(separator: ".").map(String.init)
        guard let first = parts.first else { return }
        var object: [String: JSONValue]
        if case .object(let o) = self { object = o } else { object = [:] }
        if parts.count == 1 {
            object[first] = value
        } else {
            var inner = object[first] ?? .object([:])
            inner.set(value, at: parts.dropFirst().joined(separator: "."))
            object[first] = inner
        }
        self = .object(object)
    }

    public var double: Double? { if case .number(let n) = self { return n }; return nil }
    public var isNull: Bool { self == .null }
}
