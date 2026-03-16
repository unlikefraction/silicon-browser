"use client";

import { useEffect, useRef, useCallback } from "react";
import Matter from "matter-js";

const CHAR_SET = "@#%&8B$WMQO0Xkdpbqo*+~=:-.` ";
const COLS = 120;
const FONT_RATIO = 0.55;

interface AsciiChar {
  char: string;
  x: number; // center x on screen
  y: number; // center y on screen
  col: number;
  row: number;
  isRightEye: boolean; // for wink targeting
}

export function Intro({ onDone }: { onDone: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const phaseRef = useRef<string>("loading");
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    const canvas = canvasRef.current!;
    const container = containerRef.current!;
    const W = window.innerWidth;
    const H = window.innerHeight;
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext("2d")!;

    // Load font then start
    const font = new FontFace("ApfelGrotezk", "url(/fonts/ApfelGrotezk-Regular.woff2)");
    font.load().then((loaded) => {
      document.fonts.add(loaded);
      startSequence();
    });

    let asciiChars: AsciiChar[] = [];
    let charSize = 0;
    let rafId = 0;
    let engine: Matter.Engine | null = null;
    let bodies: { body: Matter.Body; char: string }[] = [];
    let nameText = "";
    let nameOpacity = 0;

    function renderLogoToAscii(text: string): AsciiChar[] {
      const offscreen = document.createElement("canvas");
      const offCtx = offscreen.getContext("2d")!;
      const fontSize = 300;
      offCtx.font = fontSize + "px ApfelGrotezk";
      const metrics = offCtx.measureText(text);
      const textW = metrics.width;

      offscreen.width = textW + 40;
      offscreen.height = fontSize + 40;

      offCtx.fillStyle = "#000";
      offCtx.fillRect(0, 0, offscreen.width, offscreen.height);
      offCtx.fillStyle = "#fff";
      offCtx.font = fontSize + "px ApfelGrotezk";
      offCtx.textBaseline = "top";
      offCtx.fillText(text, 20, 10);

      const cols = COLS;
      const cellW = offscreen.width / cols;
      const cellH = cellW / FONT_RATIO;
      const rows = Math.floor(offscreen.height / cellH);
      const imgData = offCtx.getImageData(0, 0, offscreen.width, offscreen.height);

      // Calculate screen positioning (centered)
      charSize = Math.max(6, Math.floor(W / cols * 0.85));
      const gridPixelW = cols * charSize;
      const gridPixelH = rows * charSize * 1.15;
      const offsetX = (W - gridPixelW) / 2;
      const offsetY = (H - gridPixelH) / 2;

      // Find the midpoint of the "/" to split left eye vs right eye
      const slashCol = Math.floor(cols * 0.45); // approximate

      const chars: AsciiChar[] = [];
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const sx = Math.floor(c * cellW);
          const sy = Math.floor(r * cellH);
          const ex = Math.min(Math.floor((c + 1) * cellW), offscreen.width);
          const ey = Math.min(Math.floor((r + 1) * cellH), offscreen.height);

          let sum = 0;
          let count = 0;
          for (let y = sy; y < ey; y++) {
            for (let x = sx; x < ex; x++) {
              const idx = (y * offscreen.width + x) * 4;
              sum += imgData.data[idx];
              count++;
            }
          }
          const avg = count > 0 ? sum / count : 0;
          const brightness = avg / 255;
          const charIdx = Math.floor((1 - brightness) * (CHAR_SET.length - 1));
          const ch = CHAR_SET[charIdx] || " ";

          if (ch.trim() === "") continue; // skip spaces

          chars.push({
            char: ch,
            x: offsetX + c * charSize + charSize / 2,
            y: offsetY + r * charSize * 1.15 + charSize / 2,
            col: c,
            row: r,
            isRightEye: c > slashCol,
          });
        }
      }
      return chars;
    }

    function drawAscii(chars: AsciiChar[], winking: boolean) {
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = "#1a1a1a";
      ctx.fillRect(0, 0, W, H);
      ctx.font = charSize + "px monospace";
      ctx.fillStyle = "#ede8e0";
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";

      for (const c of chars) {
        let ch = c.char;
        // Wink: flatten right eye chars
        if (winking && c.isRightEye && "@#%&8BWMQO0Xkdpbq".includes(ch)) {
          ch = "-";
        }
        ctx.fillText(ch, c.x, c.y);
      }
    }

    function startPhysics() {
      engine = Matter.Engine.create({ gravity: { x: 0, y: 1.8 } });

      const t = 60;
      const walls = [
        Matter.Bodies.rectangle(W / 2, H + t / 2, W + 200, t, { isStatic: true }),
        Matter.Bodies.rectangle(-t / 2, H / 2, t, H * 3, { isStatic: true }),
        Matter.Bodies.rectangle(W + t / 2, H / 2, t, H * 3, { isStatic: true }),
      ];
      Matter.Composite.add(engine.world, walls);

      bodies = [];
      for (const c of asciiChars) {
        const angle = Math.random() * Math.PI * 2;
        const force = 0.015 + Math.random() * 0.035;

        const body = Matter.Bodies.rectangle(c.x, c.y, charSize * 0.8, charSize * 1.0, {
          restitution: 0.4,
          friction: 0.6,
          frictionAir: 0.005,
        });

        Matter.Body.applyForce(body, { x: c.x, y: c.y }, {
          x: Math.cos(angle) * force,
          y: Math.sin(angle) * force - 0.025,
        });
        Matter.Body.setAngularVelocity(body, (Math.random() - 0.5) * 0.4);

        bodies.push({ body, char: c.char });
        Matter.Composite.add(engine.world, body);
      }

      // Physics render loop — never stops until sequence ends
      function render() {
        if (!engine) return;
        Matter.Engine.update(engine, 1000 / 60);

        ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = "#1a1a1a";
        ctx.fillRect(0, 0, W, H);

        ctx.font = charSize + "px monospace";
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

        // Draw name text over physics
        if (nameText) {
          ctx.save();
          ctx.globalAlpha = nameOpacity;
          ctx.font = "21px ApfelGrotezk, sans-serif";
          ctx.fillStyle = "#ede8e0";
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.letterSpacing = "4px";
          ctx.fillText(nameText, W / 2, H / 2);
          ctx.restore();
        }

        rafId = requestAnimationFrame(render);
      }
      render();
    }

    async function startSequence() {
      // 1. Render "°/°" as ASCII
      asciiChars = renderLogoToAscii("°/°");

      // 2. Show logo
      drawAscii(asciiChars, false);
      await sleep(1200);

      // 3. Wink (close right eye)
      drawAscii(asciiChars, true);
      await sleep(400);

      // 4. Open eye again
      drawAscii(asciiChars, false);
      await sleep(500);

      // 5. BLAST — start physics
      startPhysics();

      // 6. After 2.5s, fade in "UNLIKEFRACTION" (physics keeps running)
      await sleep(2500);

      nameText = "UNLIKEFRACTION";
      // Fade in over ~800ms (handled in render loop via opacity)
      for (let i = 0; i <= 20; i++) {
        nameOpacity = i / 20;
        await sleep(40);
      }

      // 7. Hold for 1s
      await sleep(1000);

      // 8. Melt sequence — fast, many stages, 50ms per step
      const meltStages = [
        "UNLIKEFRACTION",
        "UNLIKEFRACTIO*",
        "UNLIKEFRACTI**",
        "UNLIKEFRACT***",
        "UNLIKEFRAC****",
        "UNLIKEFRA*****",
        "UNLIKEFR******",
        "UNLIKEF*******",
        "UNLIKE********",
        "UNLIK*********",
        "UNLI**********",
        "UNL***********",
        "UN************",
        "U*************",
        "**************",
        "++++++++++++++",
        "~~~~~~~~~~~~~~",
        "==============",
        "::::::::::::::",
        "--------------",
        "...............",
        "              ",
        "",
      ];

      for (const stage of meltStages) {
        nameText = stage;
        await sleep(50);
      }

      // 9. Fade out name
      for (let i = 20; i >= 0; i--) {
        nameOpacity = i / 20;
        await sleep(20);
      }

      await sleep(300);

      // 10. Done — transition out
      cancelAnimationFrame(rafId);
      if (engine) Matter.Engine.clear(engine);
      engine = null;
      onDoneRef.current();
    }

    function sleep(ms: number) {
      return new Promise((resolve) => setTimeout(resolve, ms));
    }

    return () => {
      cancelAnimationFrame(rafId);
      if (engine) {
        Matter.Engine.clear(engine);
        engine = null;
      }
    };
  }, []);

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
      <canvas
        ref={canvasRef}
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%" }}
      />
    </div>
  );
}
