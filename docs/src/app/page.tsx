"use client";

import { useEffect, useRef, useState } from "react";

const ASCII_ART = `
 ███████╗██╗██╗     ██╗ ██████╗ ██████╗ ███╗   ██╗
 ██╔════╝██║██║     ██║██╔════╝██╔═══██╗████╗  ██║
 ███████╗██║██║     ██║██║     ██║   ██║██╔██╗ ██║
 ╚════██║██║██║     ██║██║     ██║   ██║██║╚██╗██║
 ███████║██║███████╗██║╚██████╗╚██████╔╝██║ ╚████║
 ╚══════╝╚═╝╚══════╝╚═╝ ╚═════╝ ╚═════╝ ╚═╝  ╚═══╝

 ██████╗ ██████╗  ██████╗ ██╗    ██╗███████╗███████╗██████╗
 ██╔══██╗██╔══██╗██╔═══██╗██║    ██║██╔════╝██╔════╝██╔══██╗
 ██████╔╝██████╔╝██║   ██║██║ █╗ ██║███████╗█████╗  ██████╔╝
 ██╔══██╗██╔══██╗██║   ██║██║███╗██║╚════██║██╔══╝  ██╔══██╗
 ██████╔╝██║  ██║╚██████╔╝╚███╔███╔╝███████║███████╗██║  ██║
 ╚═════╝ ╚═╝  ╚═╝ ╚═════╝  ╚══╝╚══╝ ╚══════╝╚══════╝╚═╝  ╚═╝
`.trim();

const HERO_LOGO = `
    ╔═══╗
    ║ S ║
    ╚═══╝`;

export default function Home() {
  const [phase, setPhase] = useState<"ascii" | "tagline" | "main">("ascii");
  const [falling, setFalling] = useState(false);
  const [copied, setCopied] = useState(false);
  const [skillCopied, setSkillCopied] = useState(false);
  const asciiRef = useRef<HTMLPreElement>(null);

  const handleIntroClick = () => {
    if (falling) return;
    setFalling(true);

    // Wrap each character in a span and make them fall with random delays/rotations
    if (asciiRef.current) {
      const text = asciiRef.current.textContent || "";
      let html = "";
      for (let i = 0; i < text.length; i++) {
        const char = text[i];
        if (char === "\n") {
          html += "<br/>";
        } else if (char === " ") {
          html += " ";
        } else {
          const delay = Math.random() * 800;
          const rotate = (Math.random() - 0.5) * 180;
          const escaped = char === "<" ? "&lt;" : char === ">" ? "&gt;" : char === "&" ? "&amp;" : char;
          html += '<span class="falling" style="animation-delay:' + delay + 'ms;--rotate:' + rotate + 'deg">' + escaped + '</span>';
        }
      }
      asciiRef.current.innerHTML = html;
    }

    // Show tagline after chars fall
    setTimeout(() => {
      setPhase("tagline");
    }, 1200);

    // Transition to main
    setTimeout(() => {
      setPhase("main");
    }, 3200);
  };

  const copyInstall = () => {
    navigator.clipboard.writeText(
      "npm install -g silicon-browser && silicon-browser install"
    );
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const copySkill = async () => {
    try {
      const resp = await fetch(
        "https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md"
      );
      const text = await resp.text();
      await navigator.clipboard.writeText(text);
      setSkillCopied(true);
      setTimeout(() => setSkillCopied(false), 2000);
    } catch {
      window.open(
        "https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md",
        "_blank"
      );
    }
  };

  return (
    <>
      {/* Phase 1: ASCII Art Intro */}
      {phase !== "main" && (
        <div
          id="intro"
          className={phase === "tagline" ? "fade-out" : ""}
          onClick={handleIntroClick}
        >
          <div style={{ textAlign: "center" }}>
            <pre id="ascii-art" ref={asciiRef}>
              {ASCII_ART}
            </pre>
            {!falling && (
              <p
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: "14px",
                  color: "#525252",
                  marginTop: "3rem",
                  animation: "fadeUp 0.5s ease forwards 0.8s",
                  opacity: 0,
                }}
              >
                click to enter
              </p>
            )}
          </div>
        </div>
      )}

      {/* Phase 2: Tagline */}
      {(phase === "tagline" || phase === "main") && phase !== "main" && (
        <div id="tagline" className="visible">
          <h2>
            the most reliable browser CLI
            <br />
            for your <em>silicon</em>
          </h2>
        </div>
      )}

      {/* Phase 3: Main Content */}
      <div id="main-content" className={phase === "main" ? "visible" : ""}>
        {/* Hero */}
        <div className="hero">
          <pre className="hero-logo">{HERO_LOGO}</pre>
          <h1 className="hero-title">
            silicon<span>-</span>browser
          </h1>
          <p className="hero-subtitle">
            Stealth-first headless browser CLI for AI agents.
            CloakBrowser engine. Profiles with pinned fingerprints.
            Push/clone sessions anywhere.
          </p>
          <div className="install-box">
            <code>
              <span className="prompt">$ </span>npm install -g silicon-browser
            </code>
            <code>
              <span className="prompt">$ </span>silicon-browser install
            </code>
            <code className="comment"># that&apos;s it. stealth is on by default.</code>
            <button onClick={copyInstall}>{copied ? "copied!" : "copy"}</button>
          </div>
          <div className="hero-actions">
            <a
              href="https://github.com/unlikefraction/silicon-browser"
              className="btn btn-primary"
              target="_blank"
            >
              GitHub
            </a>
            <a href="#stealth" className="btn btn-secondary">
              how it works
            </a>
            <a href="#skill" className="btn btn-secondary">
              skill file
            </a>
          </div>
        </div>

        <hr className="divider" />

        {/* Stealth */}
        <section id="stealth">
          <h2>
            passes everything.
          </h2>
          <p>
            Stock Playwright gets flagged in milliseconds. Silicon Browser walks right past
            Cloudflare, DataDome, BrowserScan, and Shopify&apos;s login page like a real person.
          </p>

          <table className="detection-table">
            <thead>
              <tr>
                <th>detection check</th>
                <th>stock playwright</th>
                <th>silicon browser</th>
              </tr>
            </thead>
            <tbody>
              {[
                ["BrowserScan WebDriver", "fail", "pass"],
                ["BrowserScan CDP", "fail", "pass"],
                ["BrowserScan Headless Chrome", "fail", "pass"],
                ["BrowserScan Navigator", "fail", "pass"],
                ["bot.sannysoft.com (all checks)", "fail", "pass"],
                ["Shopify login page", "fail", "pass"],
                ["Cloudflare Turnstile", "fail", "pass"],
              ].map(([name, stock, sb]) => (
                <tr key={name}>
                  <td>{name}</td>
                  <td className="fail">{stock}</td>
                  <td className="pass">{sb}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <h3>four layers. zero config.</h3>
          <div className="arch-grid">
            <div className="arch-card">
              <div className="layer-num">layer 01</div>
              <h4>CloakBrowser</h4>
              <p>
                33 C++ patches compiled into Chromium. TLS, WebGL, Canvas, Audio
                — all undetectable at the engine level.
              </p>
            </div>
            <div className="arch-card">
              <div className="layer-num">layer 02</div>
              <h4>Chrome Flags</h4>
              <p>
                AutomationControlled disabled. WebRTC leak prevention. GPU
                rasterization. Fingerprint seed per profile.
              </p>
            </div>
            <div className="arch-card">
              <div className="layer-num">layer 03</div>
              <h4>JS Injection</h4>
              <p>
                Defense-in-depth: window.chrome, Permissions API, CDP artifact
                cleanup, navigator.connection.
              </p>
            </div>
            <div className="arch-card">
              <div className="layer-num">layer 04</div>
              <h4>HTTP Headers</h4>
              <p>
                Sec-Fetch-*, Accept, Accept-Language — matching real Chrome.
                No version mismatches.
              </p>
            </div>
          </div>
        </section>

        <hr className="divider" />

        {/* How it works */}
        <section>
          <h2>ref-based. not selector-based.</h2>
          <p>
            Every interactive element gets a ref like @e1, @e2. No CSS selectors.
            No XPath. Just snapshot, see refs, interact. 93% fewer tokens than
            screenshot-based approaches.
          </p>
          <div className="code-block">
            <code>
              <span className="prompt">$ </span>silicon-browser open https://shopify.com
              {"\n"}
              <span className="prompt">$ </span>silicon-browser snapshot -i
              {"\n\n"}
              <span className="output">@e1</span>{"  link \"Home\"\n"}
              <span className="output">@e2</span>{"  button \"Sign In\"\n"}
              <span className="output">@e3</span>{"  input \"Email\"\n"}
              <span className="output">@e4</span>{"  button \"Continue\"\n\n"}
              <span className="prompt">$ </span>silicon-browser fill <span className="string">@e3</span> <span className="string">&quot;hello@example.com&quot;</span>
              {"\n"}
              <span className="prompt">$ </span>silicon-browser click <span className="string">@e4</span>
            </code>
          </div>
        </section>

        <hr className="divider" />

        {/* Profiles */}
        <section>
          <h2>profiles. like a real person.</h2>
          <p>
            Each profile has its own cookies, history, and pinned fingerprint seed.
            Same profile always looks like the same person to websites. Different profile,
            different identity. Incognito for throwaway sessions.
          </p>
          <div className="code-block">
            <code>
              <span className="comment"># default profile — always there</span>
              {"\n"}
              <span className="prompt">$ </span>silicon-browser open https://example.com
              {"\n\n"}
              <span className="comment"># named profiles for different identities</span>
              {"\n"}
              <span className="prompt">$ </span>silicon-browser <span className="flag">--profile work</span> open https://shopify.com
              {"\n"}
              <span className="prompt">$ </span>silicon-browser <span className="flag">--profile personal</span> open https://github.com
              {"\n\n"}
              <span className="comment"># incognito — throwaway, random fingerprint, no traces</span>
              {"\n"}
              <span className="prompt">$ </span>silicon-browser <span className="flag">--incognito</span> open https://example.com
              {"\n\n"}
              <span className="prompt">$ </span>silicon-browser profile list
              {"\n"}
              <span className="output">{"● silicon   [23 MB] (fingerprint: 89993)"}</span>
              {"\n"}
              <span className="output">{"● work      [9 MB]  (fingerprint: 23430)"}</span>
              {"\n"}
              <span className="output">{"● personal  [9 MB]  (fingerprint: 42809)"}</span>
            </code>
          </div>
        </section>

        <hr className="divider" />

        {/* Push/Clone */}
        <section>
          <h2>push / clone.</h2>
          <p>
            Login on your laptop. Use the session on your server.
            No SSH keys. No cloud accounts. Just a URL and a 6-digit code.
            Works across the internet — auto-tunneled.
          </p>
          <div className="flow">
{`YOUR LAPTOP                              YOUR SERVER
┌──────────────────────────┐              ┌──────────────────────┐
│                          │              │                      │
│  `}<span className="highlight">silicon-browser push work</span>{`   │              │                      │
│                          │              │                      │
│  ● Serving 'work'        │              │                      │
│    Public: `}<span className="highlight">https://a1b2.lhr.life</span>{`        │                      │
│    OTP:    `}<span className="highlight">483921</span>{`         │──────────►  │  `}<span className="highlight">silicon-browser clone</span>{`  │
│                          │              │    `}<span className="highlight">https://a1b2.lhr.life</span>{`
│  `}<span className="green">✓ Sent!</span>{`                  │              │    OTP: `}<span className="highlight">483921</span>{`         │
│                          │              │  `}<span className="green">✓ Cloned!</span>{`              │
└──────────────────────────┘              └──────────────────────┘`}
          </div>
          <p style={{ color: "#525252", fontSize: "14px" }}>
            AES-256-GCM encrypted. Auto-tunneled via localhost.run. Server shuts down after one transfer.
          </p>
        </section>

        <hr className="divider" />

        {/* Skill File */}
        <section id="skill">
          <div className="skill-download">
            <h3>SKILL.md</h3>
            <p>
              593 lines. Everything an AI agent needs to use silicon-browser.
              Download it. Drop it in your agent&apos;s context. Done.
            </p>
            <div className="actions">
              <button className="btn btn-primary" onClick={copySkill}>
                {skillCopied ? "copied to clipboard!" : "copy to clipboard"}
              </button>
              <a
                href="https://raw.githubusercontent.com/unlikefraction/silicon-browser/main/SKILL.md"
                className="btn btn-secondary"
                target="_blank"
              >
                download raw
              </a>
              <a
                href="https://github.com/unlikefraction/silicon-browser/blob/main/SKILL.md"
                className="btn btn-secondary"
                target="_blank"
              >
                view on github
              </a>
            </div>
          </div>
        </section>

        <hr className="divider" />

        {/* Commands */}
        <section>
          <h2>50+ commands.</h2>
          <div className="code-block" style={{ fontSize: "13px" }}>
            <code>
              <span className="comment"># navigate</span>{"\n"}
              silicon-browser open &lt;url&gt;{"\n"}
              silicon-browser back / forward / reload{"\n"}
              silicon-browser close{"\n\n"}

              <span className="comment"># see</span>{"\n"}
              silicon-browser snapshot -i{"\n"}
              silicon-browser screenshot{"\n"}
              silicon-browser get text @e1{"\n"}
              silicon-browser get url{"\n\n"}

              <span className="comment"># interact</span>{"\n"}
              silicon-browser click @e1{"\n"}
              silicon-browser fill @e2 &quot;text&quot;{"\n"}
              silicon-browser select @e3 &quot;option&quot;{"\n"}
              silicon-browser press Enter{"\n"}
              silicon-browser scroll down{"\n"}
              silicon-browser upload @e1 ./file.pdf{"\n\n"}

              <span className="comment"># wait</span>{"\n"}
              silicon-browser wait @e1{"\n"}
              silicon-browser wait --load networkidle{"\n"}
              silicon-browser wait 2000{"\n\n"}

              <span className="comment"># profiles</span>{"\n"}
              silicon-browser --profile work open &lt;url&gt;{"\n"}
              silicon-browser --incognito open &lt;url&gt;{"\n"}
              silicon-browser profile list{"\n\n"}

              <span className="comment"># sync</span>{"\n"}
              silicon-browser push &lt;name&gt;{"\n"}
              silicon-browser clone &lt;url&gt;{"\n"}
              silicon-browser pull &lt;name&gt;{"\n\n"}

              <span className="comment"># eval</span>{"\n"}
              silicon-browser eval &quot;document.title&quot;{"\n"}
              silicon-browser eval --stdin &lt;&lt;&apos;EOF&apos;{"\n"}
              {"  "}document.querySelectorAll(&apos;a&apos;).length{"\n"}
              EOF
            </code>
          </div>
          <p>
            <a href="https://github.com/unlikefraction/silicon-browser/blob/main/SKILL.md" style={{ color: "#22d3ee" }} target="_blank">
              full command reference in SKILL.md →
            </a>
          </p>
        </section>

        <hr className="divider" />

        {/* Works with */}
        <section style={{ textAlign: "center" }}>
          <h2>works with everything.</h2>
          <p style={{ maxWidth: "600px", margin: "0 auto 2rem" }}>
            Claude Code, Cursor, GitHub Copilot, OpenAI Codex, Google Gemini,
            silicon-stemcell, and any agent that can run shell commands.
          </p>
          <div className="install-box" style={{ margin: "0 auto", opacity: 1, animation: "none" }}>
            <code>
              <span className="prompt">$ </span>npm install -g silicon-browser
            </code>
            <code>
              <span className="prompt">$ </span>silicon-browser install
            </code>
            <button onClick={copyInstall}>{copied ? "copied!" : "copy"}</button>
          </div>
        </section>

        {/* Footer */}
        <footer>
          <p>
            built on{" "}
            <a href="https://github.com/vercel-labs/agent-browser" target="_blank">
              agent-browser
            </a>{" "}
            +{" "}
            <a href="https://cloakbrowser.dev/" target="_blank">
              cloakbrowser
            </a>
            {"  ·  "}
            <a href="https://github.com/unlikefraction/silicon-browser" target="_blank">
              github
            </a>
            {"  ·  "}
            <a href="https://npmjs.com/package/silicon-browser" target="_blank">
              npm
            </a>
            {"  ·  "}
            <a href="https://unlikefraction.com" target="_blank">
              unlikefraction
            </a>
          </p>
        </footer>
      </div>
    </>
  );
}
