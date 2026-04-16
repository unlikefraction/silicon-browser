"use client";

import { useState } from "react";
import { Intro } from "@/components/intro";

export default function Home() {
  const [showIntro, setShowIntro] = useState(true);
  const [copied, setCopied] = useState(false);
  const [skillCopied, setSkillCopied] = useState(false);

  const copyInstall = () => {
    navigator.clipboard.writeText("npm install -g silicon-browser && silicon-browser install");
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const copySkill = async () => {
    try {
      const r = await fetch("https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md");
      await navigator.clipboard.writeText(await r.text());
      setSkillCopied(true);
      setTimeout(() => setSkillCopied(false), 2000);
    } catch {
      window.open("https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md", "_blank");
    }
  };

  return (
    <>
      {/* ── INTRO SEQUENCE ── */}
      {showIntro && <Intro onDone={() => setShowIntro(false)} />}

      {/* ── MAIN SITE ── */}
      <div className={"main-site" + (!showIntro ? " visible" : "")}>

        {/* Hero */}
        <div className="hero">
          <h1 className="hero-brand">
            silicon
            <br />
            browser.
          </h1>
          <div className="hero-sub">
            <div className="hero-tagline">
              stealth-first browser cli<br />
              for ai agents
            </div>
            <nav className="hero-nav">
              <a href="#stealth">stealth</a>
              <a href="#profiles">profiles</a>
              <a href="#sync">push / clone</a>
              <a href="#skill">skill file</a>
              <a href="https://github.com/unlikefraction/silicon-browser" target="_blank">github</a>
            </nav>
          </div>
        </div>

        {/* Install bar */}
        <div className="install-bar">
          <code>
            <span className="dollar">$ </span>
            npm install -g silicon-browser && silicon-browser install
          </code>
          <button className="copy-btn" onClick={copyInstall}>
            {copied ? "copied" : "copy"}
          </button>
        </div>

        {/* Stealth */}
        <section className="section" id="stealth">
          <div className="section-label">stealth</div>
          <h2>passes<br />everything.</h2>
          <p>
            CloakBrowser&apos;s 33 C++ patches at the Chromium source level.
            Not JavaScript injection — binary-level stealth that anti-bot
            systems cannot detect because there is nothing to detect.
          </p>

          <div className="detect-grid">
            <div className="dg-header">check</div>
            <div className="dg-header">playwright</div>
            <div className="dg-header">silicon</div>
            {[
              ["BrowserScan WebDriver", "Robot", "Normal"],
              ["BrowserScan CDP", "Robot", "Normal"],
              ["BrowserScan Headless Chrome", "Robot", "Normal"],
              ["BrowserScan Navigator", "Robot", "Normal"],
              ["bot.sannysoft.com", "Fail", "Pass"],
              ["Shopify login", "Blocked", "Works"],
              ["Cloudflare Turnstile", "Fail", "Pass"],
            ].map(([name, stock, sb]) => (
              <>
                <div key={name} className="dg-cell">{name}</div>
                <div className={"dg-cell fail"}>{stock}</div>
                <div className={"dg-cell pass"}>{sb}</div>
              </>
            ))}
          </div>

          <div className="layers">
            {[
              ["01", "CloakBrowser", "33 C++ patches in Chromium. TLS, WebGL, Canvas, Audio — undetectable at engine level."],
              ["02", "Chrome Flags", "AutomationControlled off. WebRTC sealed. Fingerprint seed per profile."],
              ["03", "JS Injection", "window.chrome, Permissions API, CDP cleanup. Defense-in-depth."],
              ["04", "HTTP Headers", "Sec-Fetch-*, Accept headers. Coherent with actual browser version."],
            ].map(([num, title, desc]) => (
              <div key={num} className="layer-card">
                <div className="num">layer {num}</div>
                <h4>{title}</h4>
                <p>{desc}</p>
              </div>
            ))}
          </div>
        </section>

        {/* Refs */}
        <section className="section">
          <div className="section-label">interaction</div>
          <h2>refs, not<br />selectors.</h2>
          <p>
            Every element gets a ref like @e1, @e2. No CSS selectors. No XPath.
            Snapshot, see refs, interact. 93% fewer tokens.
          </p>
          <div className="terminal">
            <span className="dim">$ </span>silicon-browser open https://shopify.com{"\n"}
            <span className="dim">$ </span>silicon-browser snapshot -i{"\n\n"}
            <span className="accent">@e1</span>{"  link \"Home\"\n"}
            <span className="accent">@e2</span>{"  button \"Sign In\"\n"}
            <span className="accent">@e3</span>{"  input \"Email\"\n"}
            <span className="accent">@e4</span>{"  button \"Continue\"\n\n"}
            <span className="dim">$ </span>silicon-browser fill <span className="accent">@e3</span> &quot;hello@example.com&quot;{"\n"}
            <span className="dim">$ </span>silicon-browser click <span className="accent">@e4</span>{"\n"}
            <span className="green">{"✓ "}</span>Log in — Shopify
          </div>
        </section>

        {/* Profiles */}
        <section className="section" id="profiles">
          <div className="section-label">identity</div>
          <h2>profiles.<br />like a real person.</h2>
          <p>
            Each profile gets its own cookies, storage, and pinned fingerprint.
            Same profile = same identity. Different profile = different person.
            Incognito = no traces.
          </p>
          <div className="terminal">
            <span className="dim"># default &quot;silicon&quot; profile — always there</span>{"\n"}
            <span className="dim">$ </span>silicon-browser open https://example.com{"\n\n"}
            <span className="dim"># named profiles</span>{"\n"}
            <span className="dim">$ </span>silicon-browser <span className="accent">--profile work</span> open https://shopify.com{"\n"}
            <span className="dim">$ </span>silicon-browser <span className="accent">--profile personal</span> open https://github.com{"\n\n"}
            <span className="dim"># throwaway</span>{"\n"}
            <span className="dim">$ </span>silicon-browser <span className="accent">--incognito</span> open https://example.com{"\n\n"}
            <span className="dim">$ </span>silicon-browser profile list{"\n"}
            <span className="green">{"● "}</span>silicon{"   [23 MB]  (fingerprint: 89993)\n"}
            <span className="green">{"● "}</span>work{"      [9 MB]   (fingerprint: 23430)\n"}
            <span className="green">{"● "}</span>personal{"  [9 MB]   (fingerprint: 42809)"}
          </div>
        </section>

        {/* Push/Clone */}
        <section className="section" id="sync">
          <div className="section-label">sync</div>
          <h2>push /<br />clone.</h2>
          <p>
            Login on your laptop. Use the session on your server. No SSH keys.
            No cloud. Just a URL and a 6-digit OTP. Auto-tunneled.
          </p>
          <div className="flow-box">
{`YOUR LAPTOP                              YOUR SERVER
┌──────────────────────────┐              ┌──────────────────────┐
│                          │              │                      │
│  `}<span className="hl">silicon-browser push work</span>{`   │              │                      │
│                          │              │                      │
│  ● Serving 'work'        │              │                      │
│    Public: `}<span className="hl">https://a1.lhr.life</span>{`           │                      │
│    OTP:    `}<span className="hl">483921</span>{`         │──────────►  │  `}<span className="hl">silicon-browser clone</span>{`  │
│                          │              │    `}<span className="hl">https://a1.lhr.life</span>{`  │
│  ✓ Sent!                 │              │    OTP: `}<span className="hl">483921</span>{`         │
│                          │              │  ✓ Cloned!             │
└──────────────────────────┘              └──────────────────────┘`}
          </div>
        </section>

        {/* Skill */}
        <section className="section" id="skill">
          <div className="section-label">for agents</div>
          <div className="skill-box">
            <h3>SKILL.md</h3>
            <p>
              593 lines. Everything an AI agent needs.
              Drop it in your agent&apos;s context.
            </p>
            <div className="skill-actions">
              <button className="btn-dark" onClick={copySkill}>
                {skillCopied ? "copied to clipboard" : "copy to clipboard"}
              </button>
              <a href="https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md" className="btn-outline" target="_blank">
                download
              </a>
              <a href="https://github.com/unlikefraction/silicon-browser/blob/main/SKILL.md" className="btn-outline" target="_blank">
                view on github
              </a>
            </div>
          </div>
        </section>

        {/* Commands */}
        <section className="section">
          <div className="section-label">commands</div>
          <h2>50+</h2>
          <div className="terminal" style={{ fontSize: "13px" }}>
            <span className="dim"># navigate</span>{"\n"}
            silicon-browser open &lt;url&gt;{"\n"}
            silicon-browser back / forward / reload{"\n"}
            silicon-browser close{"\n\n"}
            <span className="dim"># see</span>{"\n"}
            silicon-browser snapshot -i{"\n"}
            silicon-browser screenshot{"\n"}
            silicon-browser get text @e1{"\n\n"}
            <span className="dim"># interact</span>{"\n"}
            silicon-browser click @e1{"\n"}
            silicon-browser fill @e2 &quot;text&quot;{"\n"}
            silicon-browser select @e3 &quot;option&quot;{"\n"}
            silicon-browser press Enter{"\n"}
            silicon-browser scroll down{"\n\n"}
            <span className="dim"># sync</span>{"\n"}
            silicon-browser push &lt;name&gt;{"\n"}
            silicon-browser clone &lt;url&gt;{"\n"}
            silicon-browser pull &lt;name&gt;
          </div>
        </section>

        {/* Works with */}
        <section className="section" style={{ textAlign: "center" }}>
          <div className="section-label">compatibility</div>
          <h2>works with everything.</h2>
          <p style={{ margin: "0 auto" }}>
            Claude Code. Cursor. GitHub Copilot. OpenAI Codex.
            Google Gemini. Any agent that runs shell commands.
          </p>
        </section>

        {/* Footer */}
        <footer className="site-footer">
          <span>silicon-browser</span>
          <div className="footer-links">
            <a href="https://github.com/unlikefraction/silicon-browser" target="_blank">github</a>
            <a href="https://npmjs.com/package/silicon-browser" target="_blank">npm</a>
            <a href="https://github.com/vercel-labs/agent-browser" target="_blank">agent-browser</a>
            <a href="https://cloakbrowser.dev" target="_blank">cloakbrowser</a>
            <a href="https://unlikefraction.com" target="_blank">unlikefraction</a>
          </div>
        </footer>
      </div>
    </>
  );
}
