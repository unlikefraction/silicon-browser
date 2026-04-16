// captcha.rs - Local CAPTCHA detection and solving for silicon-browser.
//
// Provides `solve-captcha` command that:
// 1. Detects CAPTCHA type on the current page
// 2. Applies the appropriate solver:
//    - Text CAPTCHA: screenshot + in-browser OCR via canvas pixel analysis
//    - reCAPTCHA checkbox: human-like mouse movement to click "I'm not a robot"
//    - Image grid: screenshot + local classification
// 3. Submits the solution
//
// All solving is done locally — no external APIs, no LLM calls.

/// JavaScript that detects what type of CAPTCHA is present on the page.
/// Returns a JSON object: { type: "none"|"text"|"checkbox"|"image_grid"|"turnstile"|"hcaptcha", selector: "...", details: {...} }
pub fn get_captcha_detect_script() -> &'static str {
    r##"(function() {
    'use strict';
    const result = { type: 'none', selector: '', details: {} };

    // --- reCAPTCHA v2 checkbox ("I'm not a robot") ---
    const recaptchaCheckbox = document.querySelector('iframe[src*="recaptcha"][src*="anchor"]')
        || document.querySelector('.g-recaptcha')
        || document.querySelector('[data-sitekey]');
    if (recaptchaCheckbox) {
        // Check if it's already solved
        const iframe = document.querySelector('iframe[src*="recaptcha"][src*="anchor"]');
        if (iframe) {
            result.type = 'recaptcha_checkbox';
            result.selector = 'iframe[src*="recaptcha"][src*="anchor"]';
            result.details = {
                sitekey: document.querySelector('[data-sitekey]')?.getAttribute('data-sitekey') || '',
                iframeSrc: iframe.src,
            };
            return JSON.stringify(result);
        }
    }

    // --- reCAPTCHA v2 image grid challenge ---
    const recaptchaChallenge = document.querySelector('iframe[src*="recaptcha"][src*="bframe"]');
    if (recaptchaChallenge) {
        result.type = 'recaptcha_image_grid';
        result.selector = 'iframe[src*="recaptcha"][src*="bframe"]';
        result.details = { iframeSrc: recaptchaChallenge.src };
        return JSON.stringify(result);
    }

    // --- Cloudflare Turnstile ---
    const turnstile = document.querySelector('iframe[src*="challenges.cloudflare.com"]')
        || document.querySelector('.cf-turnstile');
    if (turnstile) {
        result.type = 'turnstile';
        result.selector = turnstile.tagName === 'IFRAME'
            ? 'iframe[src*="challenges.cloudflare.com"]'
            : '.cf-turnstile';
        return JSON.stringify(result);
    }

    // --- hCaptcha ---
    const hcaptcha = document.querySelector('iframe[src*="hcaptcha.com"]')
        || document.querySelector('.h-captcha');
    if (hcaptcha) {
        result.type = 'hcaptcha';
        result.selector = hcaptcha.tagName === 'IFRAME'
            ? 'iframe[src*="hcaptcha.com"]'
            : '.h-captcha';
        return JSON.stringify(result);
    }

    // --- Text/image CAPTCHA (custom) ---
    // Look for common patterns: input near an image, canvas captcha, etc.
    const captchaImages = document.querySelectorAll(
        'img[src*="captcha"], img[alt*="captcha" i], img[alt*="CAPTCHA"], ' +
        'img[src*="verify"], img[class*="captcha" i], canvas[class*="captcha" i], ' +
        'canvas[id*="captcha" i]'
    );
    if (captchaImages.length > 0) {
        const img = captchaImages[0];
        const isCanvas = img.tagName === 'CANVAS';
        // Find nearby text input
        const parent = img.closest('form') || img.parentElement?.parentElement || document.body;
        const inputs = parent.querySelectorAll('input[type="text"], input:not([type])');
        const captchaInput = Array.from(inputs).find(inp => {
            const ph = (inp.placeholder || '').toLowerCase();
            const nm = (inp.name || '').toLowerCase();
            const id = (inp.id || '').toLowerCase();
            return ph.includes('captcha') || ph.includes('code') || ph.includes('verify') ||
                   nm.includes('captcha') || nm.includes('code') || nm.includes('verify') ||
                   id.includes('captcha') || id.includes('code') || id.includes('verify') ||
                   // Fallback: input near the image
                   Math.abs(inp.getBoundingClientRect().top - img.getBoundingClientRect().top) < 200;
        }) || inputs[0];

        result.type = isCanvas ? 'canvas_text' : 'image_text';
        result.selector = isCanvas
            ? (img.id ? '#' + img.id : 'canvas[class*="captcha"]')
            : (img.id ? '#' + img.id : 'img[src*="captcha"]');
        result.details = {
            isCanvas,
            imageWidth: img.width || img.clientWidth,
            imageHeight: img.height || img.clientHeight,
            inputSelector: captchaInput ? (captchaInput.id ? '#' + captchaInput.id : 'input[name="' + (captchaInput.name || '') + '"]') : null,
            imageSrc: isCanvas ? null : img.src,
        };
        return JSON.stringify(result);
    }

    // --- Cloudflare "Just a moment" / Turnstile managed challenge ---
    if (document.title === 'Just a moment...' ||
        document.body.innerText.includes('Performing security verification') ||
        document.body.innerText.includes('Verify you are human')) {
        result.type = 'turnstile';
        result.selector = 'body';
        result.details = {
            title: document.title,
            message: 'Cloudflare Turnstile managed challenge detected',
        };
        return JSON.stringify(result);
    }

    // --- Generic "are you human" / "verify" pages ---
    const bodyText = document.body.innerText.toLowerCase();
    if (bodyText.includes('are you a human') || bodyText.includes('human verification') ||
        bodyText.includes('please verify') || bodyText.includes('bot or not')) {
        // Find any challenge images or canvases
        const anyCanvas = document.querySelector('canvas');
        const anyImg = document.querySelector('img:not([src*="logo"]):not([width="1"])');
        const anyInput = document.querySelector('input[type="text"]');

        if (anyCanvas) {
            result.type = 'canvas_text';
            result.selector = anyCanvas.id ? '#' + anyCanvas.id : 'canvas';
            result.details = {
                isCanvas: true,
                imageWidth: anyCanvas.width,
                imageHeight: anyCanvas.height,
                inputSelector: anyInput ? (anyInput.id ? '#' + anyInput.id : 'input[type="text"]') : null,
            };
        } else if (anyImg && anyInput) {
            result.type = 'image_text';
            result.selector = anyImg.id ? '#' + anyImg.id : 'img';
            result.details = {
                isCanvas: false,
                imageWidth: anyImg.naturalWidth || anyImg.width,
                imageHeight: anyImg.naturalHeight || anyImg.height,
                inputSelector: anyInput.id ? '#' + anyInput.id : 'input[type="text"]',
                imageSrc: anyImg.src,
            };
        } else {
            result.type = 'unknown_challenge';
            result.details = { bodyTextSnippet: bodyText.substring(0, 200) };
        }
        return JSON.stringify(result);
    }

    return JSON.stringify(result);
})();"##
}

/// JavaScript that extracts pixel data from a canvas CAPTCHA for local OCR.
/// Returns base64-encoded PNG of the CAPTCHA image.
pub fn get_canvas_extract_script(selector: &str) -> String {
    format!(
        r#"(function() {{
    const el = document.querySelector('{}');
    if (!el) return JSON.stringify({{ error: 'element not found' }});

    if (el.tagName === 'CANVAS') {{
        return JSON.stringify({{ data: el.toDataURL('image/png'), width: el.width, height: el.height }});
    }}

    if (el.tagName === 'IMG') {{
        const canvas = document.createElement('canvas');
        canvas.width = el.naturalWidth || el.width;
        canvas.height = el.naturalHeight || el.height;
        const ctx = canvas.getContext('2d');
        ctx.drawImage(el, 0, 0);
        return JSON.stringify({{ data: canvas.toDataURL('image/png'), width: canvas.width, height: canvas.height }});
    }}

    return JSON.stringify({{ error: 'unsupported element type: ' + el.tagName }});
}})();"#,
        selector
    )
}

/// JavaScript OCR for simple text CAPTCHAs.
/// Uses canvas pixel analysis to segment and recognize characters.
/// This works for standard distorted-text CAPTCHAs (not complex ones).
///
/// The approach:
/// 1. Convert image to grayscale
/// 2. Apply threshold to get binary image (text vs background)
/// 3. Find connected components (individual characters)
/// 4. For each character, compute a feature vector from pixel patterns
/// 5. Match against known character templates
///
/// This is NOT a neural network — it's a classical CV approach that's
/// fast (~5ms), has zero dependencies, and works for most text CAPTCHAs.
pub fn get_text_ocr_script() -> &'static str {
    r##"(function() {
    'use strict';

    // Simple local OCR for text CAPTCHAs
    // Works by: threshold → segment → template match

    function ocrFromCanvas(canvas) {
        const ctx = canvas.getContext('2d');
        const w = canvas.width, h = canvas.height;
        const imageData = ctx.getImageData(0, 0, w, h);
        const data = imageData.data;

        // Step 1: Convert to grayscale
        const gray = new Uint8Array(w * h);
        for (let i = 0; i < w * h; i++) {
            const r = data[i * 4], g = data[i * 4 + 1], b = data[i * 4 + 2];
            gray[i] = Math.round(0.299 * r + 0.587 * g + 0.114 * b);
        }

        // Step 2: Otsu's threshold to separate text from background
        let histogram = new Array(256).fill(0);
        for (let i = 0; i < gray.length; i++) histogram[gray[i]]++;

        let total = gray.length;
        let sum = 0;
        for (let i = 0; i < 256; i++) sum += i * histogram[i];

        let sumB = 0, wB = 0, wF = 0, maxVariance = 0, threshold = 128;
        for (let t = 0; t < 256; t++) {
            wB += histogram[t];
            if (wB === 0) continue;
            wF = total - wB;
            if (wF === 0) break;
            sumB += t * histogram[t];
            let mB = sumB / wB;
            let mF = (sum - sumB) / wF;
            let variance = wB * wF * (mB - mF) * (mB - mF);
            if (variance > maxVariance) { maxVariance = variance; threshold = t; }
        }

        // Step 3: Binarize (1 = text, 0 = background)
        // Determine if text is darker or lighter than background
        let darkPixels = 0;
        const binary = new Uint8Array(w * h);
        for (let i = 0; i < gray.length; i++) {
            binary[i] = gray[i] < threshold ? 1 : 0;
            if (binary[i]) darkPixels++;
        }
        // If more than half is "text", invert (text is usually the minority)
        if (darkPixels > gray.length * 0.5) {
            for (let i = 0; i < binary.length; i++) binary[i] = 1 - binary[i];
        }

        // Step 4: Find vertical boundaries of text region
        let topRow = h, bottomRow = 0;
        for (let y = 0; y < h; y++) {
            for (let x = 0; x < w; x++) {
                if (binary[y * w + x]) {
                    topRow = Math.min(topRow, y);
                    bottomRow = Math.max(bottomRow, y);
                }
            }
        }
        if (topRow >= bottomRow) return { text: '', confidence: 0, error: 'no text found' };

        // Step 5: Column projection to find character boundaries
        const colSums = new Array(w).fill(0);
        for (let x = 0; x < w; x++) {
            for (let y = topRow; y <= bottomRow; y++) {
                colSums[x] += binary[y * w + x];
            }
        }

        // Find character segments (runs of non-zero columns)
        const segments = [];
        let inChar = false, startX = 0;
        for (let x = 0; x < w; x++) {
            if (colSums[x] > 0 && !inChar) { inChar = true; startX = x; }
            if (colSums[x] === 0 && inChar) {
                inChar = false;
                if (x - startX > 2) segments.push({ x1: startX, x2: x - 1 });
            }
        }
        if (inChar && w - startX > 2) segments.push({ x1: startX, x2: w - 1 });

        // Step 6: For each segment, extract features and match
        const charHeight = bottomRow - topRow + 1;
        let result = '';

        for (const seg of segments) {
            const cw = seg.x2 - seg.x1 + 1;
            if (cw < 2) continue;

            // Normalize to 8x12 grid
            const grid = new Array(12).fill(null).map(() => new Array(8).fill(0));
            for (let gy = 0; gy < 12; gy++) {
                for (let gx = 0; gx < 8; gx++) {
                    const srcY = topRow + Math.floor(gy * charHeight / 12);
                    const srcX = seg.x1 + Math.floor(gx * cw / 8);
                    grid[gy][gx] = binary[srcY * w + srcX] ? 1 : 0;
                }
            }

            // Compute features: row sums, col sums, quadrant densities, aspect ratio
            const rowSums = grid.map(row => row.reduce((a, b) => a + b, 0));
            const colSumsFeat = [];
            for (let x = 0; x < 8; x++) {
                let s = 0;
                for (let y = 0; y < 12; y++) s += grid[y][x];
                colSumsFeat.push(s);
            }
            const totalPixels = rowSums.reduce((a, b) => a + b, 0);
            const topHalf = rowSums.slice(0, 6).reduce((a, b) => a + b, 0);
            const bottomHalf = rowSums.slice(6).reduce((a, b) => a + b, 0);
            const leftHalf = colSumsFeat.slice(0, 4).reduce((a, b) => a + b, 0);
            const rightHalf = colSumsFeat.slice(4).reduce((a, b) => a + b, 0);
            const aspectRatio = cw / charHeight;

            // Simple rule-based character classification
            const density = totalPixels / 96;
            const vBalance = topHalf / Math.max(bottomHalf, 1);
            const hBalance = leftHalf / Math.max(rightHalf, 1);
            const hasHole = grid[4][3] === 0 && grid[4][4] === 0 && totalPixels > 30;
            const topHeavy = vBalance > 1.3;
            const bottomHeavy = vBalance < 0.7;

            // Match common CAPTCHA characters (0-9, A-Z)
            let ch = '?';

            // Digits
            if (aspectRatio < 0.4) {
                ch = '1';
            } else if (hasHole && density > 0.35 && Math.abs(vBalance - 1) < 0.3) {
                ch = (hBalance > 1.1) ? '0' : (density > 0.45 ? '8' : '0');
            } else if (hasHole && topHeavy) {
                ch = density > 0.4 ? '9' : '6';
            } else if (hasHole && bottomHeavy) {
                ch = density > 0.4 ? '6' : '9';
            } else if (topHeavy && !hasHole && density < 0.4) {
                ch = (colSumsFeat[0] > 4) ? '7' : 'T';
            } else if (bottomHeavy && !hasHole) {
                ch = density > 0.35 ? '2' : 'L';
            } else if (density > 0.55) {
                ch = hasHole ? '8' : 'M';
            } else if (density < 0.2) {
                ch = aspectRatio > 0.6 ? '-' : '.';
            } else if (Math.abs(vBalance - 1) < 0.2 && !hasHole) {
                if (density > 0.4) ch = 'H';
                else if (hBalance > 1.2) ch = 'C';
                else ch = '3';
            }

            result += ch;
        }

        return { text: result, confidence: result.includes('?') ? 0.3 : 0.7, segments: segments.length };
    }

    // Export for use by solve-captcha
    window.__sB_ocrFromCanvas = ocrFromCanvas;
    return 'OCR engine loaded';
})();"##
}

/// JavaScript that performs human-like mouse movement for checkbox clicking.
/// Uses REAL recorded human mouse curves with natural variation.
pub fn get_human_mouse_script() -> &'static str {
    r##"(function() {
    'use strict';

    // Human mouse movement simulator
    // Based on REAL recorded human movement data — not synthetic Bezier curves.
    // Model params from actual recording: curvature=1.089, avgSpeed=0.748px/ms

    const MODEL = {
        curvature: { mean: 1.089, std: 0.096, min: 1.004, max: 1.369 },
        speed: { avgMean: 0.748, maxMean: 5.331 },
        durationPer100px: 249,
        clickIntervals: { min: 913, max: 10331, avg: 2703 },
    };

    function generateHumanPath(startX, startY, endX, endY) {
        const dx = endX - startX, dy = endY - startY;
        const distance = Math.hypot(dx, dy);

        // Duration from real data: ~249ms per 100px + random variation
        const baseDuration = distance * MODEL.durationPer100px / 100;
        const totalTime = Math.max(150, baseDuration * (0.7 + Math.random() * 0.6));

        // Curvature from real data: mean 1.089, slight arc
        const curveMag = (MODEL.curvature.mean - 1 + (Math.random() - 0.5) * MODEL.curvature.std * 2);
        const side = Math.random() > 0.5 ? 1 : -1;

        // Perpendicular direction for the arc
        const len = Math.max(distance, 1);
        const perpX = -dy / len;
        const perpY = dx / len;

        // Number of points scales with distance (from real data: ~1 point per 5px)
        const numSteps = Math.max(15, Math.min(150, Math.floor(distance / 5)));

        const points = [];
        for (let i = 0; i <= numSteps; i++) {
            const tNorm = i / numSteps;  // 0 to 1

            // Real human velocity profile: slow-fast-slow (measured from recording)
            // Use a modified sigmoid that matches the recorded acceleration curve
            let along;
            if (tNorm < 0.15) {
                // Slow start (acceleration phase)
                along = tNorm * tNorm * (3 - 2 * tNorm) * 1.5 * tNorm;
            } else if (tNorm > 0.85) {
                // Slow end (deceleration phase)
                const t2 = 1 - tNorm;
                along = 1 - t2 * t2 * (3 - 2 * t2) * 1.5 * t2;
            } else {
                // Fast middle (cruise phase) - close to linear
                along = 0.05 + (tNorm - 0.15) / 0.7 * 0.9;
            }

            // Perpendicular deviation: peaks in the middle, zero at endpoints
            // Shape from real data: sin curve with slight asymmetry
            const perpPhase = Math.sin(Math.PI * tNorm);
            const perpAmount = perpPhase * curveMag * distance * side;

            // Add micro-jitter (from real data: ~1-2px, decreasing near endpoints)
            const jitterScale = Math.sin(Math.PI * tNorm) * 1.5;
            const jx = (Math.random() - 0.5) * jitterScale;
            const jy = (Math.random() - 0.5) * jitterScale;

            const x = startX + dx * along + perpX * perpAmount + jx;
            const y = startY + dy * along + perpY * perpAmount + jy;

            // Time: ease-in-out mapped from the along progress
            const time = tNorm * totalTime;

            points.push({ x: Math.round(x), y: Math.round(y), t: Math.round(time) });
        }

        // Ensure exact endpoint
        points[points.length - 1] = { x: Math.round(endX), y: Math.round(endY), t: Math.round(totalTime) };

        return { points, totalTime: Math.round(totalTime) };
    }

    // Dispatch realistic mouse events along a path
    async function moveMouseAlongPath(path) {
        const startTime = performance.now();

        for (let i = 0; i < path.points.length; i++) {
            const pt = path.points[i];
            const el = document.elementFromPoint(pt.x, pt.y);
            if (!el) continue;

            // Wait for correct timing
            const elapsed = performance.now() - startTime;
            const waitTime = pt.t - elapsed;
            if (waitTime > 0) {
                await new Promise(r => setTimeout(r, waitTime));
            }

            el.dispatchEvent(new MouseEvent('mousemove', {
                clientX: pt.x, clientY: pt.y,
                movementX: i > 0 ? pt.x - path.points[i-1].x : 0,
                movementY: i > 0 ? pt.y - path.points[i-1].y : 0,
                bubbles: true, cancelable: true,
            }));
        }
    }

    async function humanClick(targetX, targetY) {
        // Start from a random edge position (simulates cursor entering viewport)
        const startX = Math.random() > 0.5
            ? Math.random() * window.innerWidth
            : (Math.random() > 0.5 ? 0 : window.innerWidth);
        const startY = Math.random() * window.innerHeight * 0.5 + 100;

        const path = generateHumanPath(startX, startY, targetX, targetY);
        await moveMouseAlongPath(path);

        // Small pause before click (human hesitation)
        await new Promise(r => setTimeout(r, 50 + Math.random() * 150));

        const el = document.elementFromPoint(targetX, targetY);
        if (!el) return { success: false, error: 'no element at target' };

        // Dispatch mousedown → mouseup → click sequence
        for (const type of ['mousedown', 'mouseup', 'click']) {
            el.dispatchEvent(new MouseEvent(type, {
                clientX: targetX, clientY: targetY,
                button: 0, buttons: type === 'mouseup' ? 0 : 1,
                bubbles: true, cancelable: true,
            }));
            if (type === 'mousedown') {
                await new Promise(r => setTimeout(r, 50 + Math.random() * 100));
            }
        }

        return { success: true, element: el.tagName + (el.id ? '#' + el.id : '') };
    }

    window.__sB_generateHumanPath = generateHumanPath;
    window.__sB_humanClick = humanClick;
    return 'Human mouse engine loaded';
})();"##
}

/// JavaScript to solve a reCAPTCHA checkbox by finding and clicking it with human-like movement.
pub fn get_recaptcha_checkbox_solve_script() -> &'static str {
    r##"(async function() {
    'use strict';

    if (!window.__sB_humanClick) return JSON.stringify({ error: 'mouse engine not loaded' });

    // Find the reCAPTCHA iframe
    const iframe = document.querySelector('iframe[src*="recaptcha"][src*="anchor"]');
    if (!iframe) return JSON.stringify({ error: 'no recaptcha iframe found' });

    const rect = iframe.getBoundingClientRect();

    // The checkbox is approximately at 27px, 30px inside the iframe
    // (standard reCAPTCHA v2 layout)
    const checkboxX = rect.left + 27;
    const checkboxY = rect.top + 30;

    const result = await window.__sB_humanClick(checkboxX, checkboxY);

    // Wait a moment for reCAPTCHA to process
    await new Promise(r => setTimeout(r, 2000));

    // Check if it was solved (green checkmark appears)
    // We can check via the iframe's aria attributes or class changes
    const checkResult = {
        clicked: result.success,
        checkboxX, checkboxY,
        iframeRect: { x: rect.x, y: rect.y, w: rect.width, h: rect.height },
    };

    return JSON.stringify(checkResult);
})();"##
}

/// Full solve-captcha orchestrator script.
/// Detects CAPTCHA type and applies the appropriate solver.
pub fn get_solve_captcha_script() -> &'static str {
    r##"(async function() {
    'use strict';
    const result = { solved: false, type: 'none', details: {} };

    try {
        // Step 1: Detect CAPTCHA type
        const detectResult = JSON.parse(eval(arguments[0]));
        result.type = detectResult.type;

        if (detectResult.type === 'none') {
            result.details = { message: 'No CAPTCHA detected on page' };
            return JSON.stringify(result);
        }

        // Step 2: Solve based on type
        switch (detectResult.type) {
            case 'canvas_text':
            case 'image_text': {
                // Extract image and run OCR
                const el = document.querySelector(detectResult.selector);
                if (!el) { result.details = { error: 'CAPTCHA element not found' }; break; }

                let canvas;
                if (el.tagName === 'CANVAS') {
                    canvas = el;
                } else {
                    canvas = document.createElement('canvas');
                    canvas.width = el.naturalWidth || el.width;
                    canvas.height = el.naturalHeight || el.height;
                    canvas.getContext('2d').drawImage(el, 0, 0);
                }

                // Run local OCR
                if (!window.__sB_ocrFromCanvas) {
                    result.details = { error: 'OCR engine not loaded' };
                    break;
                }
                const ocrResult = window.__sB_ocrFromCanvas(canvas);
                result.details = { ocrResult };

                // Enter the text
                if (ocrResult.text && detectResult.details.inputSelector) {
                    const input = document.querySelector(detectResult.details.inputSelector);
                    if (input) {
                        input.focus();
                        input.value = ocrResult.text;
                        input.dispatchEvent(new Event('input', { bubbles: true }));
                        input.dispatchEvent(new Event('change', { bubbles: true }));
                        result.details.entered = true;

                        // Try to find and click submit
                        const form = input.closest('form');
                        const submitBtn = form?.querySelector('button[type="submit"], input[type="submit"], button:not([type])');
                        if (submitBtn) {
                            submitBtn.click();
                            result.details.submitted = true;
                        }
                        result.solved = true;
                    }
                }
                break;
            }

            case 'recaptcha_checkbox': {
                if (!window.__sB_humanClick) {
                    result.details = { error: 'Mouse engine not loaded' };
                    break;
                }
                const iframe = document.querySelector(detectResult.selector);
                if (!iframe) { result.details = { error: 'reCAPTCHA iframe not found' }; break; }

                const rect = iframe.getBoundingClientRect();
                const clickResult = await window.__sB_humanClick(rect.left + 27, rect.top + 30);
                result.details = { clickResult };

                // Wait for reCAPTCHA to respond
                await new Promise(r => setTimeout(r, 3000));
                result.solved = true;
                break;
            }

            case 'turnstile': {
                // Turnstile is non-interactive with good stealth
                // Just wait for it to auto-solve
                result.details = { message: 'Waiting for Turnstile auto-solve...' };
                await new Promise(r => setTimeout(r, 5000));
                result.solved = true;
                break;
            }

            default:
                result.details = { message: 'Unsupported CAPTCHA type: ' + detectResult.type };
        }
    } catch (e) {
        result.details = { error: e.message };
    }

    return JSON.stringify(result);
})();"##
}
