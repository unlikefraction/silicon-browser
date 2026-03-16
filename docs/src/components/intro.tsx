"use client";

import { useEffect, useRef } from "react";
import Matter from "matter-js";

const CHAR_SET = "@#%&8B$WMQO0Xkdpbqo*+~=:-.` ";
const COLS = 100;

interface AsciiChar {
  char: string;
  x: number;
  y: number;
  col: number;
  row: number;
}

export function Intro({ onDone }: { onDone: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    const canvas = canvasRef.current!;
    const W = window.innerWidth;
    const H = window.innerHeight;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    canvas.style.width = W + "px";
    canvas.style.height = H + "px";
    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const font = new FontFace("ApfelGrotezk", "url(/fonts/ApfelGrotezk-Regular.woff2)");
    font.load().then((loaded) => {
      document.fonts.add(loaded);
      run();
    });

    let asciiOpen: AsciiChar[] = [];  // "°/°" — eyes open
    let asciiWink: AsciiChar[] = [];  // "°/-" — right eye closed
    let fontPx = 0;
    let charW = 0;
    let charH = 0;
    let rafId = 0;
    let engine: Matter.Engine | null = null;
    let bodies: { body: Matter.Body; char: string }[] = [];
    let nameText = "";
    let nameOpacity = 0;

    // Fixed canvas size so both "°/°" and "°/-" produce identically-sized grids
    let fixedCanvasW = 0;
    let fixedCanvasH = 0;

    function initCanvasSize() {
      const oc = document.createElement("canvas").getContext("2d")!;
      const fs = 400;
      oc.font = fs + "px ApfelGrotezk";
      // Use the wider text to set the fixed size
      const w1 = oc.measureText("°/°").width;
      const w2 = oc.measureText("°/-").width;
      fixedCanvasW = Math.max(w1, w2) + 60;
      fixedCanvasH = fs + 60;
    }

    function buildAscii(text: string): AsciiChar[] {
      const off = document.createElement("canvas");
      const oc = off.getContext("2d")!;
      const fs = 400;
      off.width = fixedCanvasW;
      off.height = fixedCanvasH;
      oc.fillStyle = "#000";
      oc.fillRect(0, 0, off.width, off.height);
      oc.fillStyle = "#fff";
      oc.font = fs + "px ApfelGrotezk";
      oc.textBaseline = "top";
      oc.fillText(text, 30, 15);

      // Determine on-screen char dimensions FIRST so we know the exact display ratio
      const cols = COLS;
      fontPx = Math.max(7, Math.floor(W * 0.7 / cols));
      ctx.font = fontPx + "px monospace";
      charW = ctx.measureText("@").width;
      charH = fontPx * 1.2;

      // Sample brightness into grid using the SAME aspect ratio as the display chars
      // This is the key: cellH/cellW must equal charH/charW for circles to stay round
      const displayRatio = charH / charW;
      const cellW = off.width / cols;
      const cellH = cellW * displayRatio;
      const rows = Math.floor(off.height / cellH);
      const img = oc.getImageData(0, 0, off.width, off.height);

      const gridW = cols * charW;
      const gridH = rows * charH;
      const ox = (W - gridW) / 2;
      const oy = (H - gridH) / 2;

      const chars: AsciiChar[] = [];
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const sx = Math.floor(c * cellW);
          const sy = Math.floor(r * cellH);
          const ex = Math.min(Math.floor((c + 1) * cellW), off.width);
          const ey = Math.min(Math.floor((r + 1) * cellH), off.height);
          let sum = 0, cnt = 0;
          for (let y = sy; y < ey; y++) {
            for (let x = sx; x < ex; x++) {
              sum += img.data[(y * off.width + x) * 4];
              cnt++;
            }
          }
          const bright = cnt > 0 ? sum / cnt / 255 : 0;
          const ci = Math.floor((1 - bright) * (CHAR_SET.length - 1));
          const ch = CHAR_SET[ci] || " ";
          if (ch.trim() === "") continue;
          chars.push({
            char: ch,
            x: ox + c * charW + charW / 2,
            y: oy + r * charH + charH / 2,
            col: c, row: r,
          });
        }
      }
      return chars;
    }

    function draw(chars: AsciiChar[]) {
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = "#1a1a1a";
      ctx.fillRect(0, 0, W, H);
      ctx.font = fontPx + "px monospace";
      ctx.fillStyle = "#ede8e0";
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";
      for (const c of chars) {
        ctx.fillText(c.char, c.x, c.y);
      }
    }

    function blast() {
      engine = Matter.Engine.create({ gravity: { x: 0, y: 2 } });
      const t = 60;
      Matter.Composite.add(engine.world, [
        Matter.Bodies.rectangle(W / 2, H + t / 2, W + 200, t, { isStatic: true }),
        Matter.Bodies.rectangle(-t / 2, H / 2, t, H * 3, { isStatic: true }),
        Matter.Bodies.rectangle(W + t / 2, H / 2, t, H * 3, { isStatic: true }),
      ]);

      bodies = [];
      for (const c of asciiOpen) {
        const a = Math.random() * Math.PI * 2;
        const f = 0.01 + Math.random() * 0.03;
        const body = Matter.Bodies.rectangle(c.x, c.y, charW, charH, {
          restitution: 0.35, friction: 0.5, frictionAir: 0.003,
        });
        Matter.Body.applyForce(body, { x: c.x, y: c.y }, {
          x: Math.cos(a) * f, y: Math.sin(a) * f - 0.02,
        });
        Matter.Body.setAngularVelocity(body, (Math.random() - 0.5) * 0.3);
        bodies.push({ body, char: c.char });
        Matter.Composite.add(engine.world, body);
      }

      function render() {
        if (!engine) return;
        Matter.Engine.update(engine, 1000 / 60);
        ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = "#1a1a1a";
        ctx.fillRect(0, 0, W, H);
        ctx.font = fontPx + "px monospace";
        ctx.fillStyle = "#ede8e0";
        ctx.textBaseline = "middle";
        ctx.textAlign = "center";
        for (const b of bodies) {
          const { x, y } = b.body.position;
          ctx.save();
          ctx.translate(x, y);
          ctx.rotate(b.body.angle);
          ctx.fillText(b.char, 0, 0);
          ctx.restore();
        }
        if (nameText) {
          ctx.save();
          ctx.globalAlpha = nameOpacity;
          ctx.font = "21px ApfelGrotezk, sans-serif";
          ctx.fillStyle = "#ede8e0";
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.fillText(nameText, W / 2, H / 2);
          ctx.restore();
        }
        rafId = requestAnimationFrame(render);
      }
      render();
    }

    async function run() {
      // Build TWO separate ASCII grids from TWO different renders
      initCanvasSize();
      asciiOpen = buildAscii("°/°");   // eyes open
      asciiWink = buildAscii("°/-");   // right eye winked (rendered from the actual font)

      // Show logo — eyes open
      draw(asciiOpen);
      await sleep(1200);

      // Wink — show the "°/-" version (completely different shape from font)
      draw(asciiWink);
      await sleep(400);

      // Open again
      draw(asciiOpen);
      await sleep(500);

      // Blast (physics keeps running from here) — uses the open version
      blast();

      // Fade in name
      await sleep(2500);
      nameText = "UNLIKEFRACTION";
      for (let i = 0; i <= 20; i++) { nameOpacity = i / 20; await sleep(40); }

      await sleep(800);

      // Melt — fast, many stages, 50ms
      const stages = [
        "UNLIKEFRACTION",
        "UNLIKEFRACTIO~",
        "UNLIKEFRACTI~~",
        "UNLIKEFRACT~~~",
        "UNLIKEFRAC~~~~",
        "UNLIKEFRA~~~~~",
        "UNLIKEFR~~~~~~",
        "UNLIKEF~~~~~~~",
        "UNLIKE~~~~~~~~",
        "UNLIK~~~~~~~~~",
        "UNLI~~~~~~~~~~",
        "UNL~~~~~~~~~~~",
        "UN~~~~~~~~~~~~",
        "U~~~~~~~~~~~~~",
        "~~~~~~~~~~~~~~",
        "++++++++++++++",
        "**************",
        ":::::::::::::::",
        "---------------",
        "______________",
        "...............",
        "               ",
      ];
      for (const s of stages) { nameText = s; await sleep(50); }

      nameOpacity = 0;
      await sleep(400);

      cancelAnimationFrame(rafId);
      if (engine) Matter.Engine.clear(engine);
      engine = null;
      onDoneRef.current();
    }

    function sleep(ms: number) {
      return new Promise((r) => setTimeout(r, ms));
    }

    return () => {
      cancelAnimationFrame(rafId);
      if (engine) { Matter.Engine.clear(engine); engine = null; }
    };
  }, []);

  return (
    <div ref={containerRef} style={{
      position: "fixed", inset: 0, zIndex: 100, background: "#1a1a1a", overflow: "hidden",
    }}>
      <canvas ref={canvasRef} style={{ position: "absolute", top: 0, left: 0 }} />
    </div>
  );
}
