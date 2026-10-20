// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import {
  Activity,
  ArrowDownToLine,
  ArrowUpRight,
  Check,
  ChevronDown,
  CircleHelp,
  Cpu,
  Ellipsis,
  HardDrive,
  LayoutGrid,
  Moon,
  Plus,
  Search,
  Settings2,
  ShieldCheck,
  Snowflake,
  Sparkles,
  SquareTerminal,
  Sun,
  X,
} from "lucide-react";

type Page = "Overview" | "Virtual machines" | "Images" | "Recipes";

const navItems: { label: Page; icon: typeof LayoutGrid }[] = [
  { label: "Overview", icon: LayoutGrid },
  { label: "Virtual machines", icon: SquareTerminal },
  { label: "Images", icon: HardDrive },
  { label: "Recipes", icon: Snowflake },
];

function App() {
  const [page, setPage] = useState<Page>("Overview");
  const [query, setQuery] = useState("");
  const [notice, setNotice] = useState("Preview data · VM controls are not connected yet");

  const showPreview = (action: string) => {
    setNotice(`${action} will be available when the omaf UI protocol is connected.`);
    window.setTimeout(() => setNotice("Preview data · VM controls are not connected yet"), 3500);
  };

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <a className="brand" href="#" onClick={() => setPage("Overview")}>
          <span className="brand-mark"><Snowflake size={19} strokeWidth={2.2} /></span>
          <span className="brand-name">oma<span>flake</span></span>
        </a>

        <div className="workspace-label">WORKSPACE</div>
        <nav className="primary-nav" aria-label="Main navigation">
          {navItems.map(({ label, icon: Icon }) => (
            <button className={`nav-item ${page === label ? "active" : ""}`} key={label} onClick={() => setPage(label)}>
              <Icon size={17} strokeWidth={1.8} />
              <span>{label}</span>
              {label === "Virtual machines" && <span className="nav-count">2</span>}
            </button>
          ))}
        </nav>

        <div className="sidebar-bottom">
          <div className="host-card">
            <div className="host-icon"><span className="status-dot green" /></div>
            <div className="host-copy">
              <strong>Development host</strong>
              <span>NixOS · x86_64</span>
            </div>
            <ChevronDown size={15} className="muted-icon" />
          </div>
          <button className="nav-item quiet" onClick={() => showPreview("Settings")}><Settings2 size={17} /><span>Settings</span></button>
          <button className="nav-item quiet" onClick={() => showPreview("Help")}><CircleHelp size={17} /><span>Help & feedback</span></button>
          <div className="profile-row">
            <div className="avatar">J</div>
            <div className="profile-copy"><strong>Local profile</strong><span>Personal workspace</span></div>
            <Ellipsis size={18} className="muted-icon" />
          </div>
        </div>
      </aside>

      <section className="main-panel">
        <header className="topbar">
          <div className="breadcrumbs"><span>Workspace</span><span className="crumb-slash">/</span><strong>{page}</strong></div>
          <div className="topbar-actions">
            <label className="searchbox">
              <Search size={15} />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search anything..." />
              <kbd>⌘ K</kbd>
            </label>
            <button className="icon-button" aria-label="Theme settings" onClick={() => showPreview("Theme settings")}><Sun size={17} /></button>
            <button className="icon-button" aria-label="Activity" onClick={() => showPreview("Activity")}><Activity size={17} /><i /></button>
          </div>
        </header>

        <div className="page-content">
          <section className="welcome-row">
            <div>
              <div className="eyebrow"><Sparkles size={13} /> YOUR PRIVATE VM WORKSPACE <span className="preview-pill">PREVIEW DATA</span></div>
              <h1>{page === "Overview" ? "Good morning, Jonas" : page}</h1>
              <p className="welcome-subtitle">A clean slate for every idea. Pick up where you left off.</p>
            </div>
            <button className="button primary" onClick={() => showPreview("New virtual machine")}><Plus size={16} /> New virtual machine</button>
          </section>

          <section className="stats-grid" aria-label="Workspace summary">
            <StatCard label="Running now" value="02" detail="of 08 virtual machines" icon={Activity} tone="mint" />
            <StatCard label="Images ready" value="12" detail="across 4 recipes" icon={HardDrive} tone="violet" />
            <StatCard label="Host status" value="Healthy" detail="KVM acceleration active" icon={ShieldCheck} tone="amber" />
            <StatCard label="Storage used" value="84.6" unit="GB" detail="of 256 GB available" icon={Cpu} tone="blue" />
          </section>

          <section className="content-grid">
            <div className="section-card vm-card">
              <div className="section-heading">
                <div><h2>Recent virtual machines</h2><p>Your environments, ready when you are</p></div>
                <button className="text-link" onClick={() => setPage("Virtual machines")}>View all <ArrowUpRight size={14} /></button>
              </div>
              <div className="vm-list">
                <VMRow name="research-lab" recipe="Research · NixOS" status="Running" color="mint" uptime="2h 14m" onAction={() => showPreview("Open research-lab")} />
                <VMRow name="clean-room" recipe="Minimal · NixOS" status="Running" color="mint" uptime="38m" onAction={() => showPreview("Open clean-room")} />
                <VMRow name="graphics-test" recipe="Lab · NixOS" status="Stopped" color="gray" uptime="Yesterday" onAction={() => showPreview("Start graphics-test")} />
              </div>
              <button className="create-row" onClick={() => showPreview("Create virtual machine")}><span><Plus size={15} /></span> Create from an image</button>
            </div>

            <div className="section-card activity-card">
              <div className="section-heading">
                <div><h2>Recent activity</h2><p>What’s happening in your workspace</p></div>
                <button className="icon-button small" aria-label="More activity options"><Ellipsis size={18} /></button>
              </div>
              <div className="timeline">
                <ActivityItem icon={Check} tone="mint" title="Image build completed" detail="nixos/research · 12 min ago" />
                <ActivityItem icon={Moon} tone="violet" title="VM shut down" detail="graphics-test · 1 hour ago" />
                <ActivityItem icon={ArrowDownToLine} tone="blue" title="Recipe installed" detail="omaflake/nixos/lab · 3 hours ago" />
              </div>
              <button className="activity-link" onClick={() => showPreview("Full activity history")}>See activity history <ArrowUpRight size={14} /></button>
            </div>
          </section>

          <section className="section-card recipes-card">
            <div className="section-heading">
              <div><h2>Pick up a recipe</h2><p>Thoughtful starting points for your next environment</p></div>
              <button className="text-link" onClick={() => setPage("Recipes")}>Browse recipes <ArrowUpRight size={14} /></button>
            </div>
            <div className="recipe-grid">
              <RecipeCard name="Minimal" category="NIXOS · ESSENTIALS" description="A quiet, clean workspace with just the essentials." badge="READY TO BUILD" shade="orange" onAction={() => showPreview("Build Minimal image")} />
              <RecipeCard name="Research" category="NIXOS · SCIENCE" description="A considered toolkit for research and deep work." badge="IMAGE AVAILABLE" shade="violet" onAction={() => showPreview("Create from Research")} />
              <RecipeCard name="Lab" category="NIXOS · DEVELOPMENT" description="Tools for building, testing, and trying something new." badge="READY TO BUILD" shade="blue" onAction={() => showPreview("Build Lab image")} />
            </div>
          </section>

          <footer className="footer-row">
            <div className="host-footer"><span className="status-dot green" /> All systems operational <span className="footer-divider">·</span> KVM enabled</div>
            <div>omaflake <span className="footer-divider">·</span> Preview 0.1.0</div>
          </footer>
        </div>
      </section>

      {notice && <div className="toast" role="status"><span className="toast-dot" />{notice}<button aria-label="Dismiss message" onClick={() => setNotice("")}><X size={14} /></button></div>}
    </main>
  );
}

function StatCard({ label, value, unit, detail, icon: Icon, tone }: {
  label: string; value: string; unit?: string; detail: string; icon: typeof Activity; tone: string;
}) {
  return <article className="stat-card"><div className={`stat-icon ${tone}`}><Icon size={17} /></div><span className="stat-label">{label}</span><div className="stat-value">{value}<small>{unit}</small></div><span className="stat-detail">{detail}</span></article>;
}

function VMRow({ name, recipe, status, color, uptime, onAction }: {
  name: string; recipe: string; status: string; color: string; uptime: string; onAction: () => void;
}) {
  return <div className="vm-row"><div className={`vm-glyph ${color}`}><Snowflake size={18} /></div><div className="vm-copy"><strong>{name}</strong><span>{recipe}</span></div><div className="vm-state"><span className={`status-dot ${color === "mint" ? "green" : "gray"}`} /><span>{status}</span></div><span className="vm-uptime">{uptime}</span><button className="vm-action" onClick={onAction} aria-label={`${status === "Running" ? "Open" : "Start"} ${name}`}>{status === "Running" ? <ArrowUpRight size={16} /> : <Moon size={15} />}</button></div>;
}

function ActivityItem({ icon: Icon, tone, title, detail }: { icon: typeof Check; tone: string; title: string; detail: string }) {
  return <div className="timeline-item"><div className={`timeline-icon ${tone}`}><Icon size={15} /></div><div><strong>{title}</strong><span>{detail}</span></div></div>;
}

function RecipeCard({ name, category, description, badge, shade, onAction }: {
  name: string; category: string; description: string; badge: string; shade: string; onAction: () => void;
}) {
  return <article className={`recipe-card ${shade}`}><div className="recipe-art"><div className="art-orbit orbit-one" /><div className="art-orbit orbit-two" /><div className="art-core"><Snowflake size={28} strokeWidth={1.35} /></div><span className="art-star star-one">✳</span><span className="art-star star-two">✦</span></div><div className="recipe-copy"><span className="recipe-category">{category}</span><h3>{name}</h3><p>{description}</p><div className="recipe-bottom"><span className="recipe-badge"><span />{badge}</span><button className="recipe-action" onClick={onAction} aria-label={`${badge === "READY TO BUILD" ? "Build" : "Create from"} ${name}`}><ArrowUpRight size={16} /></button></div></div></article>;
}

export default App;
