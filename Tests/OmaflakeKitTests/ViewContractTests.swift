// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// The page models of the presentation model (docs/api.md §10, plan B121): every golden file in
/// crates/omaflake-core/testdata/api/views/ decodes with this app's view types – strictly – and the values the app
/// draws arrive. Part of the contract tests (`APIContractTests`): a new file without a check here fails
/// `everyViewFileIsChecked`.
extension APIContractTests {
    static let viewFolder = folder.appendingPathComponent("views")

    func page(_ name: String, as page: String) throws -> PageModel {
        try PageModel.decode(page: page, from: Data(contentsOf: Self.viewFolder.appendingPathComponent("\(name).json")))
    }

    func viewObject(_ name: String) throws -> NSDictionary {
        try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: Self.viewFolder.appendingPathComponent("\(name).json"))) as? NSDictionary)
    }

    /// Every view golden file and its check
    var viewChecks: [String: () throws -> Void] {
        [
            "home": {
                guard case .home(let h) = try page("home", as: "home") else { throw Unexpected() }
                #expect(h.actions.map(\.id) == ["doctor", "credentials", "settings", "dataFolder", "quit"] && h.actions[0].badge == 3)
                #expect(h.actions[4].confirm?.text.args == [.string("research")] && h.firstTab == "vms")
                #expect(h.actions[1].icon == "key")
                #expect(h.tabs.map(\.id) == ["vms", "images", "recipes", "looks", "activity"] && h.tabs[0].badge?.count == 1)
                #expect(h.tabs[1].badge?.kind == "working" && h.tabs[2].badge?.tone == .warning && h.tabs[3].badge == nil)
                // The window's title is data; the header's short labels
                #expect(h.title == ViewText(verbatim: "omaflake"))
                #expect(h.actions.map { $0.shortLabel?.key } == ["Doctor", nil, nil, "Data Folder", "Quit"])
                #expect(h.actions[3].label.key == "Show Data Folder in Finder" && !h.actions.contains { $0.primary })
            },
            "home.linux": {
                // Host wording is the core's: the host-neutral variant on Linux
                guard case .home(let h) = try page("home.linux", as: "home") else { throw Unexpected() }
                #expect(h.actions[3].label.key == "Show Data Folder" && h.actions[3].shortLabel?.key == "Data Folder")
            },
            "vms": {
                guard case .vms(let p) = try page("vms", as: "vms") else { throw Unexpected() }
                #expect(p.head.title.key == "Virtual Machines" && p.head.count == 2 && p.head.of == nil)
                #expect(p.head.actions.map(\.id) == ["import", "external"] && p.head.actions[0].ask.first?.multiple == true)
                #expect(p.head.actions[0].ask.first?.check?.method == "packages.inspect" && p.head.actions[1].ask.first?.multiple == false)
                let r = try #require(p.rows.first)
                #expect(r.lamp.state == .on && r.actions.map(\.id) == ["showShare", "terminal", "stop"])
                // The running VM's default is its terminal; the other row has none
                #expect(r.actions.primary?.id == "terminal" && p.rows.last?.actions.primary == nil)
                #expect(r.actions[1].then == .terminal && r.actions[2].destructive && r.vm.name == "research")
                #expect(r.detail.switches.first?.on == true && r.detail.switchesNote != nil)
                #expect(r.detail.actions.allSatisfy { !$0.enabled && $0.reason?.key == "Shut the VM down first" })
                #expect(r.detail.info.first { $0.label.key == "Created" }?.value.args.first.map { if case .date = $0 { true } else { false } } == true)
                #expect(p.rows.last?.lamp.state == .working && p.unreadable.count == 1)
                #expect(p.head.filter?.groups.map(\.id) == ["status", "kind"] && p.head.filter?.groups[0].value == .null)
            },
            "vms.filtered": {
                guard case .vms(let p) = try page("vms.filtered", as: "vms") else { throw Unexpected() }
                #expect(p.head.count == 0 && p.head.of == 2 && p.head.filter?.active == true)
                #expect(p.head.empty?.action?.method == "ui.filter" && p.head.empty?.action?.params["filter"] == .null)
            },
            "vm": {
                guard case .vm(let p) = try page("vm", as: "vm") else { throw Unexpected() }
                #expect(p.row.name == "research" && p.title == ViewText(verbatim: "research"))
            },
            "event.view.changed": {
                let params = try viewObject("event.view.changed")
                let line = try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "method": "view.changed", "params": params])
                let event = ServerEvent.decode(method: "view.changed", line: line)
                guard case .viewChanged("vms", let echoed, .vms(let p)) = event else { throw Unexpected() }
                #expect(echoed["filter"]?["status"] == .bool(true) && p.head.of == 2)
            },
            "images": {
                guard case .images(let p) = try page("images", as: "images") else { throw Unexpected() }
                #expect(p.head.count == 2 && !p.jobs.isEmpty && p.rows.count == 2)
                // On a Mac the macOS bases are a section of their own (plan B137)
                let bases = try #require(p.bases)
                #expect(bases.rows.map(\.name) == ["mac-base-26A434"] && bases.rows[0].actions.map(\.id) == ["reveal", "remove"])
                let job = try #require(p.jobs.first)
                #expect(job.state == .running && job.fraction != nil && job.actions.first?.method == "jobs.cancel")
                #expect(job.open.method == "ui.show" && job.open.params["page"] == .string("build") && job.open.primary)
                let row = try #require(p.rows.first)
                #expect(row.actions.map(\.id) == ["reveal", "newVm"] || row.actions.map(\.id) == ["reveal", "doctor"])
                #expect(p.rows.map { $0.actions.primary?.id } == ["doctor", "newVm"])
                #expect(row.detail.actions.map(\.id) == ["export", "remove"])
            },
            "image": {
                guard case .image(let p) = try page("image", as: "image") else { throw Unexpected() }
                #expect(p.row.name == "omaflake.macos.dev@stone-lamp" && p.row.image.kind == .macos)
                #expect(p.title == ViewText(verbatim: p.row.name))
            },
            "recipes": {
                guard case .recipes(let p) = try page("recipes", as: "recipes") else { throw Unexpected() }
                #expect(p.catalogsLoaded && !p.loading && p.errors.count == 1 && p.unreadable.count == 1)
                #expect(p.head.actions.map(\.id) == ["refresh", "import"] && !p.rows.isEmpty && !p.offered.isEmpty)
                #expect(p.updates?.rows.isEmpty == false)
                let r = try #require(p.rows.first { $0.recipe.id == "omaflake.nixos.dev@velvet-tiger-orbit" })
                #expect(r.lamp.state == .working || r.lamp.state == .attention)
                #expect(!r.detail.versions.isEmpty && r.detail.versions[0].actions.map(\.id) == ["export", "remove"])
                #expect(p.head.filter?.groups.contains { $0.kind == .toggle && $0.id == "otherSystems" } == true)
                #expect(p.offeredTitle?.key == "More recipes in your sources" && p.loadingText == nil && p.labHint == nil)
                #expect(p.offered.allSatisfy { $0.actions.primary?.id == "install" })
                #expect(p.rows.map { $0.actions.primary?.id } == ["doctor", "build"])
                #expect(p.updates?.rows.allSatisfy(\.action.primary) == true)
            },
            "recipes.loading": {
                guard case .recipes(let p) = try page("recipes.loading", as: "recipes") else { throw Unexpected() }
                #expect(p.loading && p.loadingText?.key == "Looking into the recipe sources …")
                #expect(p.labHidden && p.labHint?.action?.params["page"] == .string("settings") && p.labHint?.icon == "lab")
                #expect(p.head.actions.first { $0.id == "refresh" }?.busy == true)
            },
            "newVm": {
                guard case .newVm(let p) = try page("newVm", as: "newVm") else { throw Unexpected() }
                #expect(p.title.key == "New VM" && p.valid && p.actions.map(\.id) == ["cancel", "create"])
                let fields = p.sections.flatMap(\.fields)
                let cpus = try #require(fields.first { $0.id == "cpus" })
                #expect(cpus.kind == .number && cpus.min == 1 && cpus.max == 10 && cpus.label.args == [.int(4)])
                #expect(cpus.param == "settings.cpus" && p.actions[1].then == .back && p.actions[1].method == "vms.create")
                #expect(fields.first { $0.id == "network" }?.choices.count == 3)
                // Form values are kept by `valueParam`; create is the page's primary, cancel not
                #expect(cpus.valueParam == "cpus" && fields.first { $0.id == "sharePath" }?.valueParam == "sharePath")
                #expect(fields.allSatisfy { $0.kind == .info || $0.valueParam != nil })
                #expect(p.actions.map(\.primary) == [false, true])
            },
            "newVm.macos": {
                guard case .newVm(let p) = try page("newVm.macos", as: "newVm") else { throw Unexpected() }
                let fields = p.sections.flatMap(\.fields)
                #expect(!p.valid && fields.first { $0.id == "name" }?.error != nil && fields.first { $0.id == "diskGB" }?.kind == .info)
                #expect(fields.first { $0.id == "network" }?.choices.count == 2 && !p.actions[1].enabled)
                #expect(fields.first { $0.id == "diskGB" }?.valueParam == nil && p.actions[1].primary)
            },
            "build": {
                guard case .build(let p) = try page("build", as: "build") else { throw Unexpected() }
                #expect(p.state == .running && p.progress?.quietWarning == 120 && p.actions.map(\.id) == ["cancel"])
                #expect(p.log?.job == p.job?.id && p.log?.full?.method == "ui.open" && p.log?.from == 0)
                // Cancel is not the page's default
                #expect(p.actions.primary == nil)
            },
            "build.failed": {
                guard case .build(let p) = try page("build.failed", as: "build") else { throw Unexpected() }
                #expect(p.state == .failed && p.outcome?.tone == .danger && p.outcome?.detail?.text == "Recipe \"omaflake.nixos.dev\" is not in your recipes")
                // The job's error as a text: the app shows it in its language
                #expect(p.outcome?.detail?.key == "Recipe \"%@\" is not in your recipes")
                #expect(p.actions.first?.id == "build" && p.actions.primary?.id == "build")
            },
            "build.macos": {
                guard case .build(let p) = try page("build.macos", as: "build") else { throw Unexpected() }
                // The default credential has a stored password: nothing is asked (plan B137)
                #expect(p.state == .idle && p.actions.first?.ask.isEmpty == true && p.badges.first?.id == "lab")
                #expect(p.fields?.map(\.id) == ["credential", "look"] && p.fields?.first?.valueParam == "credential")
            },
            "doctor": {
                guard case .doctor(let p) = try page("doctor", as: "doctor") else { throw Unexpected() }
                #expect(p.issues == 3 && p.sections.map(\.id) == ["requirements", "nixos", "macos"] && p.checked != nil)
                #expect(p.versions.map(\.label.key) == ["omaflake"] && p.versions[0].value.args.count == 2 && p.actions.first?.method == "doctor.check")
                let nix = try #require(p.sections[0].items.first { $0.item == .nix })
                #expect(nix.status == .ok && !nix.expanded && nix.actions.map(\.id) == ["install", "setPath"])
                // The guide: its title, its page on omaflake.dev; no `path` outside omaflake.app
                let guide = try #require(p.sections.flatMap(\.items).flatMap(\.actions).first { $0.params["page"] == .string("guide") })
                #expect(guide.params["params"]?["url"] == .string("https://omaflake.dev/linux-builder/"))
                #expect(guide.params["params"]?["title"]?["key"] == .string("Linux Builder Guide"))
                #expect(guide.params["params"]?["file"] == .string("linux-builder") && guide.params["params"]?["path"] == nil)
            },
            "macBase": {
                guard case .macBase(let p) = try page("macBase", as: "macBase") else { throw Unexpected() }
                // The IPSW is shown once: in `ipsw`, not among the fields
                // The guest user from a credential, no password asked (plan B139)
                #expect(p.fields.map(\.id) == ["credential"] && p.fields[0].kind == .choice && p.actions.first?.ask.isEmpty == true)
                #expect(p.fields.map(\.valueParam) == ["credential"] && p.actions.primary?.id == "create")
                #expect(p.base?.tone == .warning && p.ipsw.download != nil && p.ipsw.actions.map(\.method) == ["jobs.cancel"])
                // The log as the build page's: the base creation's real job
                #expect(p.log?.job == "j6" && p.log?.from == 0 && p.log?.lines == 3 && p.log?.full?.method == "ui.open")
            },
            "macBase.ipsw": {
                // A chosen IPSW: its line names it; both actions set values.ipsw of this page
                guard case .macBase(let p) = try page("macBase.ipsw", as: "macBase") else { throw Unexpected() }
                #expect(p.ipsw.text.text == "UniversalMac.ipsw" && p.ipsw.actions.map(\.id) == ["useApple", "chooseIpsw"])
                #expect(p.ipsw.actions.allSatisfy { $0.method == "ui.field" && $0.params["page"] == .string("macBase") && $0.params["valueParam"] == .string("ipsw") })
                #expect(p.ipsw.actions[0].params["value"] == .null && p.actions.first?.params["ipsw"] == .string("/Volumes/Disk/UniversalMac.ipsw"))
            },
            "credentials": {
                guard case .credentials(let p) = try page("credentials", as: "credentials") else { throw Unexpected() }
                // A row as the other lists: in use (lamp, by whom), the quick Copy Password, the rest when opened
                #expect(p.rows.map(\.name) == ["Work"] && p.rows[0].badges.map(\.id) == ["default"])
                #expect(p.rows[0].lamp.state == .on && p.rows[0].usedBy.count == 2 && p.rows[0].actions.map(\.id) == ["copyPassword"])
                #expect(p.rows[0].detail.actions.map(\.id) == ["edit", "password", "default", "remove"])
                #expect(p.add.method == "ui.show" && p.add.params["page"] == .string("credential"))
            },
            "credential": {
                // New: all fields, the password asked twice on Create
                guard case .credential(let p) = try page("credential", as: "credential") else { throw Unexpected() }
                #expect(p.id == nil && p.locked == nil && p.fields.map(\.id) == ["name", "user", "fullName", "email"])
                #expect(p.fields.allSatisfy { $0.enabled && $0.valueParam == $0.param })
                let create = try #require(p.actions.last)
                #expect(create.method == "credentials.add" && create.ask.first?.kind == .password && create.then == .back)
            },
            "baseCredential": {
                // A base from before bases kept their credential: the same form, one choice (plan B139)
                guard case .credential(let p) = try page("baseCredential", as: "baseCredential") else { throw Unexpected() }
                #expect(p.fields.map(\.id) == ["credential"] && p.actions.last?.method == "bases.assign")
                #expect(p.actions.last?.params["name"] == .string("mac-base-26A434"))
            },
            "credential.edit": {
                // Used by an image: only the name can change, the other fields are greyed out with the reason
                guard case .credential(let p) = try page("credential.edit", as: "credential") else { throw Unexpected() }
                #expect(p.id == "c-1" && p.locked != nil && p.fields.filter(\.enabled).map(\.id) == ["name"])
                #expect(p.actions.last?.method == "credentials.update" && p.actions.last?.params["id"] == .string("c-1"))
            },
            "looks": {
                // The Looks tab (plan B141): rows with a colour strip and usage, the default without a quick action,
                // offered looks with Install
                guard case .looks(let p) = try page("looks", as: "looks") else { throw Unexpected() }
                #expect(p.rows.map(\.name) == ["Rosé Pine Dawn", "omaflake"] && p.rows[0].swatches.count == 11)
                #expect(p.rows[0].badges.map(\.id) == ["default", "official"] && p.rows[0].actions.isEmpty)
                #expect(p.offered.first?.actions.first?.method == "looks.install" && p.catalogsLoaded)
            },
            "credentials.linux": {
                guard case .credentials(let p) = try page("credentials.linux", as: "credentials") else { throw Unexpected() }
                #expect(p.footer.key?.hasPrefix("Passwords are kept in the desktop's keyring") == true)
            },
            "settings": {
                guard case .settings(let p) = try page("settings", as: "settings") else { throw Unexpected() }
                #expect(p.sources.rows.count == 2 && p.sources.add.fields.map(\.id) == ["input", "private"])
                #expect(p.sources.add.fields.map(\.valueParam) == ["input", "private"] && p.sources.addAction.primary)
                #expect(p.general.fields.allSatisfy { $0.valueParam == nil })
                #expect(p.sources.addAction.ask.first?.kind == .password && p.sources.rows[0].actions[1].on != nil)
                #expect(p.general.fields.allSatisfy { $0.set?.method == "settings.set" })
                #expect(p.guest.fields.first { $0.id == "locale" }?.choices.first?.value == .null)
                #expect(p.dataFolder.actions.map(\.id) == ["default", "move"])
                #expect(p.dataFolder.actions[1].confirm?.title.args == [.param("target")])
                #expect(p.guest.fields.first { $0.id == "locale" }?.choices.first?.label.key == "Like this Mac – %@")
            },
            "doctor.linux": {
                guard case .doctor(let p) = try page("doctor.linux", as: "doctor") else { throw Unexpected() }
                #expect(p.sections.map(\.id) == ["requirements", "nixos"])
                let kvm = try #require(p.sections[0].items.first { $0.item == .kvm })
                #expect(kvm.status == .limited && kvm.title.key == "KVM (hardware virtualization)")
            },
            "vm.linux": {
                guard case .vm(let p) = try page("vm.linux", as: "vm") else { throw Unexpected() }
                let accelerator = try #require(p.row.detail.info.first { $0.label.key == "Accelerator" })
                #expect(accelerator.value.text == "TCG" && accelerator.note?.key?.hasPrefix("emulated") == true)
            },
            "settings.linux": {
                guard case .settings(let p) = try page("settings.linux", as: "settings") else { throw Unexpected() }
                #expect(p.guest.fields.first { $0.id == "locale" }?.choices.first?.label.key == "Like this computer – %@")
            },
            "activity": {
                guard case .activity(let p) = try page("activity", as: "activity") else { throw Unexpected() }
                #expect(p.entries.map(\.id) == [2, 1] && p.entries[0].notification?.title.key == "VM ready")
                #expect(p.entries[1].level == .error && p.entries[1].text.key == nil && p.head.actions.first?.id == "clear")
            },
            "import": {
                guard case .importStep(let p) = try page("import", as: "import") else { throw Unexpected() }
                #expect(p.valid && p.kind == .vm && p.action?.confirm?.input?.default == "research-2")
                #expect(p.title.key == "Import" && p.action?.primary == true && p.inspection.reason == nil)
            },
            "views.get.params": {
                let ref = PageRef("newVm", .object([
                    "image": .string("omaflake.nixos.dev@cedar-mango-lift"),
                    "values": .object(["name": .string("research-2"), "cpus": .number(2), "memoryGB": .number(4),
                                       "audio": .bool(false), "start": .bool(false)]),
                ]))
                let sent = try #require(try JSONSerialization.jsonObject(with: JSONEncoder.api.encode(ref)) as? NSDictionary)
                #expect(sent == (try viewObject("views.get.params")))
            },
            "views.subscribe.params": {
                struct Subscribe: Encodable { let pages: [PageRef] }
                let pages = [PageRef("home", nil),
                             PageRef("vms", .object(["filter": .object(["status": .bool(true), "kind": .string("nixos")])])),
                             PageRef("build", .object(["recipe": .string("omaflake.nixos.dev@velvet-tiger-orbit")]))]
                let sent = try #require(try JSONSerialization.jsonObject(with: JSONEncoder.api.encode(Subscribe(pages: pages))) as? NSDictionary)
                #expect(sent == (try viewObject("views.subscribe.params")))
            },
        ]
    }

    struct Unexpected: Error {}

    @Test func everyViewFileIsChecked() throws {
        let files = try FileManager.default.contentsOfDirectory(atPath: Self.viewFolder.path).filter { $0.hasSuffix(".json") }
        #expect(!files.isEmpty, "no golden files in \(Self.viewFolder.path)")
        let names = Set(files.map { String($0.dropLast(5)) })
        #expect(names == Set(viewChecks.keys), "view files without a check: \(names.subtracting(viewChecks.keys).sorted()); checks without a file: \(Set(viewChecks.keys).subtracting(names).sorted())")
    }

    @Test func everyViewFileDecodes() {
        for (name, check) in viewChecks.sorted(by: { $0.key < $1.key }) {
            do { try check() } catch { Issue.record("views/\(name).json: \(error)") }
        }
    }

    /// Every text of every page renders, in English with the core's own date and size format, to the core's `text`
    @Test func everyTextRendersAsTheCoreWordsIt() throws {
        let utc = DateFormatter()
        utc.locale = Locale(identifier: "en_US_POSIX")
        utc.timeZone = TimeZone(identifier: "UTC")
        utc.dateFormat = "yyyy-MM-dd HH:mm 'UTC'"
        let english = ViewTextRenderer(translate: { $0 }, date: { utc.string(from: $0) }, bytes: { b in
            let d = Double(b)
            return d >= 1e9 ? String(format: "%.1f GB", d / 1e9) : d >= 1e6 ? String(format: "%.1f MB", d / 1e6)
                : d >= 1e3 ? String(format: "%.1f KB", d / 1e3) : "\(b) bytes"
        })
        var count = 0
        func walk(_ v: Any) throws {
            if let o = v as? [String: Any] {
                if o["text"] is String, o.keys.allSatisfy({ ["key", "text", "args"].contains($0) }) {
                    let data = try JSONSerialization.data(withJSONObject: o)
                    let t = try JSONDecoder.api.decode(ViewText.self, from: data)
                    let params = t.args.contains(.param("target")) ? ["target": "<target>"] : [:]
                    #expect(english.render(t, params: params) == t.text, "\(t)")
                    count += 1
                }
                for x in o.values { try walk(x) }
            } else if let a = v as? [Any] {
                for x in a { try walk(x) }
            }
        }
        for file in try FileManager.default.contentsOfDirectory(atPath: Self.viewFolder.path) where file.hasSuffix(".json") {
            try walk(JSONSerialization.jsonObject(with: Data(contentsOf: Self.viewFolder.appendingPathComponent(file))))
        }
        #expect(count > 500)
    }

    /// The app's German table translates the views' keys with placeholders that fit their args (else the English
    /// text would show)
    @Test func germanTranslationsFitTheArgs() throws {
        let table = Self.folder.appendingPathComponent("../../../../App/Resources/de.lproj/Localizable.strings").standardizedFileURL
        let de = try #require(try PropertyListSerialization.propertyList(from: Data(contentsOf: table), format: nil) as? [String: String])
        var checked = 0
        func walk(_ v: Any) throws {
            if let o = v as? [String: Any] {
                if let key = o["key"] as? String, o["text"] is String, let translation = de[key] {
                    let t = try JSONDecoder.api.decode(ViewText.self, from: JSONSerialization.data(withJSONObject: o))
                    let ints = t.args.map { if case .int = $0 { true } else { false } }
                    #expect(ViewTextRenderer.fill(translation, t.args.map { _ in "x" }, ints: ints) != nil, "\(key) → \(translation)")
                    checked += 1
                }
                for x in o.values { try walk(x) }
            } else if let a = v as? [Any] {
                for x in a { try walk(x) }
            }
        }
        for file in try FileManager.default.contentsOfDirectory(atPath: Self.viewFolder.path) where file.hasSuffix(".json") {
            try walk(JSONSerialization.jsonObject(with: Data(contentsOf: Self.viewFolder.appendingPathComponent(file))))
        }
        #expect(checked > 300)
    }

    @Test func translationsMayReorderAndFallBack() {
        let t = ViewText(key: "Upgrade “%@” to %@?", args: [.string("a"), .string("b")], text: "Upgrade “a” to b?")
        let de = ViewTextRenderer(translate: { $0 == "Upgrade “%@” to %@?" ? "Auf %2$@ umstellen: „%1$@“?" : $0 })
        #expect(de.render(t) == "Auf b umstellen: „a“?")
        let broken = ViewTextRenderer(translate: { _ in "%lld %@" })
        #expect(broken.render(t) == "Upgrade “a” to b?")
        let nested = ViewText(key: "Open the Doctor first: %@", args: [.text(ViewText(key: "Nix"))], text: "x")
        #expect(ViewTextRenderer(translate: { $0 == "Nix" ? "NIX" : $0 }).render(nested) == "Open the Doctor first: NIX")
        #expect(ViewTextRenderer.english.render(ViewText(key: "100%% sure")) == "100% sure")
        #expect(ViewTextRenderer.english.render(ViewText(key: "%lld VMs", args: [.int(3)])) == "3 VMs")
    }

    @Test func paramsGoWhereTheyArePut() {
        var p = JSONValue.object(["guest": .object(["user": .string("a")])])
        p.set(.string("de_CH"), at: "guest.locale")
        p.set(.string("/x"), at: "params.path")
        #expect(p == .object(["guest": .object(["user": .string("a"), "locale": .string("de_CH")]),
                              "params": .object(["path": .string("/x")])]))
    }
}
