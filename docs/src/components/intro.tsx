"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import Matter from "matter-js";

// ──────────────────────────────────────────────
// The intro sequence:
// 1. Render "°/°" big in Apfel Grotezk via canvas → ASCII grid
// 2. Wink: right ° closes to - then opens again
// 3. Blast: Matter.js physics, chars explode, bounded by screen walls, pile up at bottom
// 4. "UNLIKEFRACTION" fades in at center
// 5. Text melts: each letter → U → S → * → - → _ → . → gone
// 6. Transition out
// ──────────────────────────────────────────────

const CHAR_SET = "@#%&8B$WMQO0Xkdpbqo*+~=:-.` ";
const COLS = 120;
const FONT_RATIO = 0.55; // char height/width ratio for monospace

interface CharBody {
  body: Matter.Body;
  char: string;
  col: number;
  row: number;
}

export function Intro({ onDone }: { onDone: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<"logo" | "wink" | "blast" | "name" | "melt" | "done">("logo");
  const [asciiGrid, setAsciiGrid] = useState<string[][]>([]);
  const [winkFrame, setWinkFrame] = useState(0); // 0=open, 1=closed, 2=open again
  const engineRef = useRef<Matter.Engine | null>(null);
  const bodiesRef = useRef<CharBody[]>([]);
  const rafRef = useRef<number>(0);
  const [nameText, setNameText] = useState("UNLIKEFRACTION");
  const [nameOpacity, setNameOpacity] = useState(0);

  // Step 1: Render logo to ASCII
  useEffect(() => {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d")!;

    // Wait for font to load
    const font = new FontFace("ApfelGrotezk", "url(/fonts/ApfelGrotezk-Regular.woff2)");
    font.load().then((loaded) => {
      document.fonts.add(loaded);

      const text = "°/°";
      const fontSize = 300;
      ctx.font = `${fontSize}px ApfelGrotezk`;
      const metrics = ctx.measureText(text);
      const textW = metrics.width;
      const textH = fontSize;

      canvas.width = textW + 40;
      canvas.height = textH + 40;

      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.fillStyle = "#fff";
      ctx.font = `${fontSize}px ApfelGrotezk`;
      ctx.textBaseline = "top";
      ctx.fillText(text, 20, 10);

      // Sample to ASCII grid
      const cols = COLS;
      const cellW = canvas.width / cols;
      const cellH = cellW / FONT_RATIO;
      const rows = Math.floor(canvas.height / cellH);

      const grid: string[][] = [];
      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);

      for (let r = 0; r < rows; r++) {
        const row: string[] = [];
        for (let c = 0; c < cols; c++) {
          const sx = Math.floor(c * cellW);
          const sy = Math.floor(r * cellH);
          const ex = Math.min(Math.floor((c + 1) * cellW), canvas.width);
          const ey = Math.min(Math.floor((r + 1) * cellH), canvas.height);

          let sum = 0;
          let count = 0;
          for (let y = sy; y < ey; y++) {
            for (let x = sx; x < ex; x++) {
              const idx = (y * canvas.width + x) * 4;
              sum += imgData.data[idx]; // just red channel
              count++;
            }
          }
          const avg = count > 0 ? sum / count : 0;
          const brightness = avg / 255;
          const charIdx = Math.floor((1 - brightness) * (CHAR_SET.length - 1));
          row.push(CHAR_SET[charIdx] || " ");
        }
        grid.push(row);
      }

      setAsciiGrid(grid);
    });
  }, []);

  // Step 2: Wink sequence after logo appears
  useEffect(() => {
    if (asciiGrid.length === 0) return;

    const timer1 = setTimeout(() => {
      setPhase("wink");
      setWinkFrame(1); // close eye
    }, 1200);

    const timer2 = setTimeout(() => {
      setWinkFrame(2); // open eye
    }, 1700);

    const timer3 = setTimeout(() => {
      setPhase("blast"); // boom
    }, 2200);

    return () => {
      clearTimeout(timer1);
      clearTimeout(timer2);
      clearTimeout(timer3);
    };
  }, [asciiGrid]);

  // Step 3: Matter.js physics blast
  useEffect(() => {
    if (phase !== "blast" || !containerRef.current) return;

    const container = containerRef.current;
    const W = window.innerWidth;
    const H = window.innerHeight;

    const engine = Matter.Engine.create({ gravity: { x: 0, y: 1.5 } });
    engineRef.current = engine;

    // Walls (screen edges as container)
    const wallThickness = 60;
    const walls = [
      Matter.Bodies.rectangle(W / 2, H + wallThickness / 2, W + 200, wallThickness, { isStatic: true }), // floor
      Matter.Bodies.rectangle(-wallThickness / 2, H / 2, wallThickness, H * 2, { isStatic: true }), // left
      Matter.Bodies.rectangle(W + wallThickness / 2, H / 2, wallThickness, H * 2, { isStatic: true }), // right
    ];
    Matter.Composite.add(engine.world, walls);

    // Get all char positions from the rendered ASCII
    const pre = container.querySelector("pre");
    if (!pre) return;

    const charSpans = pre.querySelectorAll("span[data-char]");
    const bodies: CharBody[] = [];

    charSpans.forEach((span) => {
      const el = span as HTMLElement;
      const rect = el.getBoundingClientRect();
      const char = el.dataset.char || "";
      if (char.trim() === "") return;

      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;

      // Random explosion force
      const angle = Math.random() * Math.PI * 2;
      const force = 0.02 + Math.random() * 0.04;

      const body = Matter.Bodies.rectangle(cx, cy, rect.width, rect.height, {
        restitution: 0.3,
        friction: 0.8,
        frictionAir: 0.01,
        angle: 0,
      });

      Matter.Body.applyForce(body, { x: cx, y: cy }, {
        x: Math.cos(angle) * force,
        y: Math.sin(angle) * force - 0.03,
      });

      Matter.Body.setAngularVelocity(body, (Math.random() - 0.5) * 0.3);

      bodies.push({
        body,
        char,
        col: parseInt(el.dataset.col || "0"),
        row: parseInt(el.dataset.row || "0"),
      });

      Matter.Composite.add(engine.world, body);
    });

    bodiesRef.current = bodies;

    // Hide the pre, we'll render on canvas now
    pre.style.display = "none";

    // Render loop
    const canvas = canvasRef.current!;
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext("2d")!;

    const charSize = Math.max(8, Math.floor(W / COLS * 0.9));

    const render = () => {
      Matter.Engine.update(engine, 1000 / 60);

      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = "#1a1a1a";
      ctx.fillRect(0, 0, W, H);

      ctx.font = `${charSize}px "ApfelGrotezk", monospace`;
      ctx.fillStyle = "#ede8e0";
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";

      for (const b of bodies) {
        const { x, y } = b.body.position;
        const angle = b.body.angle;

        ctx.save();
        ctx.translate(x, y);
        ctx.rotate(angle);
        ctx.fillText(b.char, 0, 0);
        ctx.restore();
      }

      rafRef.current = requestAnimationFrame(render);
    };

    render();

    // After 2.5s, show "UNLIKEFRACTION"
    const nameTimer = setTimeout(() => {
      setPhase("name");
      setNameOpacity(1);
    }, 2500);

    return () => {
      cancelAnimationFrame(rafRef.current);
      clearTimeout(nameTimer);
      Matter.Engine.clear(engine);
    };
  }, [phase]);

  // Step 4: "UNLIKEFRACTION" melt sequence
  useEffect(() => {
    if (phase !== "name") return;

    const meltStages = ["UNLIKEFRACTION", "UUUUUUUUUUUUUU", "SSSSSSSSSSSSSS", "**************", "--------------", "______________", "..............", ""];
    let stage = 0;

    const startMelt = setTimeout(() => {
      setPhase("melt");

      const interval = setInterval(() => {
        stage++;
        if (stage < meltStages.length) {
          setNameText(meltStages[stage]);
          if (stage === meltStages.length - 1) {
            setNameOpacity(0);
          }
        } else {
          clearInterval(interval);
          setTimeout(() => {
            cancelAnimationFrame(rafRef.current);
            onDone();
          }, 400);
        }
      }, 180);
    }, 1500);

    return () => clearTimeout(startMelt);
  }, [phase, onDone]);

  // Render the ASCII as positioned spans (for Matter.js to read positions)
  const renderAscii = () => {
    if (asciiGrid.length === 0) return null;

    // For wink: replace right ° with -
    // The right eye is roughly in the right third of the grid
    const isWinking = winkFrame === 1;

    return (
      <pre
        style={{
          fontFamily: "monospace",
          fontSize: `clamp(5px, ${100 / COLS}vw, 12px)`,
          lineHeight: "1.15",
          color: "#ede8e0",
          position: "absolute",
          top: "50%",
          left: "50%",
          transform: "translate(-50%, -50%)",
          whiteSpace: "pre",
          userSelect: "none",
        }}
      >
        {asciiGrid.map((row, r) => (
          <span key={r}>
            {row.map((char, c) => {
              // Wink: find right eye area (chars in right portion that form the °)
              // Simple heuristic: right 40% of non-space chars
              let displayChar = char;
              if (isWinking && char.trim() !== "" && c > COLS * 0.55) {
                // Replace round chars with flat ones for wink effect
                if ("@#%&8BWMQO0".includes(char)) {
                  displayChar = "-";
                }
              }

              return (
                <span
                  key={`${r}-${c}`}
                  data-char={displayChar}
                  data-col={c}
                  data-row={r}
                >
                  {displayChar}
                </span>
              );
            })}
            {"\n"}
          </span>
        ))}
      </pre>
    );
  };

  return (
    <div
      ref={containerRef}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 100,
        background: "#1a1a1a",
        overflow: "hidden",
      }}
    >
      {/* ASCII logo (visible in logo/wink phases) */}
      {(phase === "logo" || phase === "wink" || phase === "blast") && renderAscii()}

      {/* Physics canvas (visible during blast) */}
      <canvas
        ref={canvasRef}
        style={{
          position: "absolute",
          inset: 0,
          display: phase === "blast" || phase === "name" || phase === "melt" ? "block" : "none",
        }}
      />

      {/* UNLIKEFRACTION text */}
      {(phase === "name" || phase === "melt") && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            zIndex: 10,
            pointerEvents: "none",
          }}
        >
          <span
            style={{
              fontFamily: "ApfelGrotezk, sans-serif",
              fontSize: "clamp(20px, 4vw, 56px)",
              letterSpacing: "0.2em",
              color: "#ede8e0",
              opacity: nameOpacity,
              transition: "opacity 0.8s ease",
            }}
          >
            {nameText}
          </span>
        </div>
      )}
    </div>
  );
}
