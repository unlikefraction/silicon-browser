// stealth.rs - Anti-detection and stealth evasion logic for silicon-browser.
//
// This module provides Chrome launch arguments, JavaScript injection scripts,
// user-agent strings, and HTTP headers designed to make automated Chrome
// instances appear indistinguishable from a regular human-operated browser.
//
// The stealth script is injected via Page.addScriptToEvaluateOnNewDocument so
// that every frame (including iframes) receives the patches before any page
// code can observe the default (detectable) values.

// Current Chrome version — update this single block when bumping.
#[allow(dead_code)]
const CHROME_MAJOR: &str = "135";
#[allow(dead_code)]
const CHROME_FULL: &str = "135.0.7049.114";
const CHROME_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.7049.114 Safari/537.36";

/// Returns additional Chrome CLI flags that reduce automation-related signals.
pub fn get_stealth_chrome_args() -> Vec<String> {
    vec![
        // Disable automation flags that leak bot detection
        "--disable-blink-features=AutomationControlled".to_string(),
        // Disable infobars (removes "Chrome is being controlled by automated software")
        "--disable-infobars".to_string(),
        // Disable WebRTC IP leak (prevents real IP exposure behind proxy)
        "--enforce-webrtc-ip-permission-check".to_string(),
        "--webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
        // GPU rendering to avoid SwiftShader detection in WebGL
        "--enable-gpu-rasterization".to_string(),
        "--enable-zero-copy".to_string(),
        // Disable the "automation controlled" Chrome flag
        "--disable-ipc-flooding-protection".to_string(),
        // Exclude the enable-automation switch
        "--disable-features=AutomationControlled,AcceptCHFrame,MediaRouter,OptimizationHints,DialMediaRouteProvider".to_string(),
        // Use real GPU angle backend (avoid SwiftShader headless detection)
        "--use-gl=angle".to_string(),
        "--use-angle=metal".to_string(),
        // Disable crash reporter (leaks automation context)
        "--disable-crash-reporter".to_string(),
    ]
}

/// Returns a comprehensive JavaScript stealth script to be injected via
/// `Page.addScriptToEvaluateOnNewDocument`. The script patches numerous
/// browser APIs so that common bot-detection libraries (e.g. FingerprintJS,
/// CreepJS, BotD) cannot distinguish the automated session from a real user.
///
/// The entire payload is wrapped in an IIFE with a top-level try/catch so
/// that a failure in one evasion does not prevent the others from running.
pub fn get_stealth_script() -> &'static str {
    r##"(function() {
    'use strict';

    // ===================================================================
    // PHASE 0: Function.prototype.toString protection
    // Must be FIRST so all subsequent patches are protected.
    // ===================================================================
    const _nativeToString = Function.prototype.toString;
    const _patchedFns = new WeakMap();
    const _markNative = (fn, name) => { _patchedFns.set(fn, `function ${name}() { [native code] }`); return fn; };
    try {
        Function.prototype.toString = function() {
            if (_patchedFns.has(this)) return _patchedFns.get(this);
            return _nativeToString.call(this);
        };
        _patchedFns.set(Function.prototype.toString, 'function toString() { [native code] }');
    } catch (_) {}

    // ===================================================================
    // (a) navigator.webdriver — must be false
    // ===================================================================
    try {
        Object.defineProperty(navigator, 'webdriver', { get: () => false, configurable: true });
        Object.defineProperty(Navigator.prototype, 'webdriver', { get: () => false, configurable: true });
    } catch (_) {}

    // ===================================================================
    // (b) navigator.plugins — mock five realistic plugins
    // ===================================================================
    try {
        const pluginData = [
            { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
            { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' },
            { name: 'Chromium PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'Chromium PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
        ];
        const makeMimeType = (p) => ({ type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format', enabledPlugin: p });
        const plugins = pluginData.map((d) => {
            const p = Object.create(Plugin.prototype);
            Object.defineProperties(p, {
                name: { value: d.name, enumerable: true }, filename: { value: d.filename, enumerable: true },
                description: { value: d.description, enumerable: true }, length: { value: 1, enumerable: true },
            });
            const mime = makeMimeType(p);
            Object.defineProperty(p, 0, { value: mime });
            p[Symbol.iterator] = function* () { yield mime; };
            return p;
        });
        const pluginArray = Object.create(PluginArray.prototype);
        plugins.forEach((p, i) => {
            Object.defineProperty(pluginArray, i, { value: p, enumerable: true });
            Object.defineProperty(pluginArray, p.name, { value: p });
        });
        Object.defineProperty(pluginArray, 'length', { value: plugins.length, enumerable: true });
        pluginArray[Symbol.iterator] = function* () { for (const p of plugins) yield p; };
        pluginArray.item = _markNative((i) => plugins[i] || null, 'item');
        pluginArray.namedItem = _markNative((n) => plugins.find((p) => p.name === n) || null, 'namedItem');
        pluginArray.refresh = _markNative(() => {}, 'refresh');
        Object.defineProperty(navigator, 'plugins', { get: () => pluginArray, configurable: true });
    } catch (_) {}

    // ===================================================================
    // (c) navigator.languages
    // ===================================================================
    try { Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'], configurable: true }); } catch (_) {}

    // ===================================================================
    // (d) navigator.hardwareConcurrency — 8 cores
    // ===================================================================
    try { Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 8, configurable: true }); } catch (_) {}

    // ===================================================================
    // (e) navigator.deviceMemory — 8 GB
    // ===================================================================
    try { Object.defineProperty(navigator, 'deviceMemory', { get: () => 8, configurable: true }); } catch (_) {}

    // ===================================================================
    // (f) navigator.platform — platform-aware
    // ===================================================================
    try {
        Object.defineProperty(navigator, 'platform', {
            get: () => {
                const ua = navigator.userAgent || '';
                if (ua.includes('Mac')) return 'MacIntel';
                if (ua.includes('Win')) return 'Win32';
                if (ua.includes('Linux')) return 'Linux x86_64';
                return 'MacIntel';
            }, configurable: true,
        });
    } catch (_) {}

    // ===================================================================
    // (g) window.chrome — convincing chrome object
    // ===================================================================
    try {
        if (!window.chrome) window.chrome = {};
        window.chrome.app = {
            isInstalled: false,
            InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
            RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
            getDetails: _markNative(() => null, 'getDetails'),
            getIsInstalled: _markNative(() => false, 'getIsInstalled'),
        };
        window.chrome.runtime = {
            OnInstalledReason: { CHROME_UPDATE: 'chrome_update', INSTALL: 'install', SHARED_MODULE_UPDATE: 'shared_module_update', UPDATE: 'update' },
            OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
            PlatformArch: { ARM: 'arm', ARM64: 'arm64', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
            PlatformNaclArch: { ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
            PlatformOs: { ANDROID: 'android', CROS: 'cros', FUCHSIA: 'fuchsia', LINUX: 'linux', MAC: 'mac', OPENBSD: 'openbsd', WIN: 'win' },
            RequestUpdateCheckStatus: { NO_UPDATE: 'no_update', THROTTLED: 'throttled', UPDATE_AVAILABLE: 'update_available' },
            connect: _markNative(() => { throw new TypeError('Error in invocation of runtime.connect'); }, 'connect'),
            sendMessage: _markNative(() => { throw new TypeError('Error in invocation of runtime.sendMessage'); }, 'sendMessage'),
            id: undefined,
        };
        window.chrome.loadTimes = _markNative(function() {
            return { commitLoadTime: Date.now()/1000-0.5, connectionInfo:'h2', finishDocumentLoadTime: Date.now()/1000-0.1, finishLoadTime: Date.now()/1000-0.05, firstPaintAfterLoadTime:0, firstPaintTime: Date.now()/1000-0.3, navigationType:'Other', npnNegotiatedProtocol:'h2', requestTime: Date.now()/1000-1, startLoadTime: Date.now()/1000-0.8, wasAlternateProtocolAvailable:false, wasFetchedViaSpdy:true, wasNpnNegotiated:true };
        }, 'loadTimes');
        window.chrome.csi = _markNative(function() {
            return { onloadT: Date.now(), pageT: Date.now()-performance.timing.navigationStart, startE: performance.timing.navigationStart, tran: 15 };
        }, 'csi');
    } catch (_) {}

    // ===================================================================
    // (h) Permissions API — return "prompt" for notifications
    // ===================================================================
    try {
        const _origQuery = Permissions.prototype.query;
        Permissions.prototype.query = _markNative(function(p) {
            if (p && p.name === 'notifications') return Promise.resolve({ state: 'prompt', onchange: null });
            return _origQuery.call(this, p);
        }, 'query');
    } catch (_) {}

    // ===================================================================
    // (i) WebGL vendor/renderer — PLATFORM-AWARE (critical fix)
    // ===================================================================
    try {
        const _getParamProto = WebGLRenderingContext.prototype.getParameter;
        const UNMASKED_VENDOR  = 0x9245;
        const UNMASKED_RENDERER = 0x9246;
        const _ua = navigator.userAgent || '';
        const _isMac = _ua.includes('Mac');
        const _vendor = _isMac ? 'Google Inc. (Apple)' : 'Google Inc. (NVIDIA)';
        const _renderer = _isMac
            ? 'ANGLE (Apple, Apple M1 Pro, OpenGL 4.1)'
            : 'ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)';

        WebGLRenderingContext.prototype.getParameter = _markNative(function(param) {
            if (param === UNMASKED_VENDOR) return _vendor;
            if (param === UNMASKED_RENDERER) return _renderer;
            return _getParamProto.call(this, param);
        }, 'getParameter');

        if (typeof WebGL2RenderingContext !== 'undefined') {
            const _getParamProto2 = WebGL2RenderingContext.prototype.getParameter;
            WebGL2RenderingContext.prototype.getParameter = _markNative(function(param) {
                if (param === UNMASKED_VENDOR) return _vendor;
                if (param === UNMASKED_RENDERER) return _renderer;
                return _getParamProto2.call(this, param);
            }, 'getParameter');
        }
    } catch (_) {}

    // ===================================================================
    // (j) iframe contentWindow proxy
    // ===================================================================
    try {
        const _origCW = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
        if (_origCW && _origCW.get) {
            Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
                get: function() {
                    const win = _origCW.get.call(this);
                    if (!win) return win;
                    try { void win.document; return win; } catch (_) {
                        return new Proxy(win, { get: (t, p) => { try { return t[p]; } catch (_) { return undefined; } } });
                    }
                }, configurable: true,
            });
        }
    } catch (_) {}

    // ===================================================================
    // (k) console.debug protection
    // ===================================================================
    try {
        const _nativeDebug = console.debug;
        Object.defineProperty(console, 'debug', { value: _nativeDebug, writable: false, configurable: false });
    } catch (_) {}

    // ===================================================================
    // (l) window.outerWidth/Height
    // ===================================================================
    try {
        if (window.outerWidth === 0) Object.defineProperty(window, 'outerWidth', { get: () => window.innerWidth + 15, configurable: true });
        if (window.outerHeight === 0) Object.defineProperty(window, 'outerHeight', { get: () => window.innerHeight + 85, configurable: true });
    } catch (_) {}

    // ===================================================================
    // (m) navigator.connection — mock NetworkInformation
    // ===================================================================
    try {
        if (!navigator.connection) {
            const ci = { rtt:50, downlink:10, effectiveType:'4g', saveData:false, onchange:null,
                addEventListener:_markNative(()=>{}, 'addEventListener'),
                removeEventListener:_markNative(()=>{}, 'removeEventListener'),
                dispatchEvent:_markNative(()=>true, 'dispatchEvent') };
            Object.defineProperty(navigator, 'connection', { get: () => ci, configurable: true });
        }
    } catch (_) {}

    // ===================================================================
    // (n) Notification.permission — "default"
    // ===================================================================
    try { Object.defineProperty(Notification, 'permission', { get: () => 'default', configurable: true }); } catch (_) {}

    // ===================================================================
    // (o) window.screen — consistent size/depth
    // ===================================================================
    try {
        Object.defineProperty(screen, 'width', { get: () => 1920, configurable: true });
        Object.defineProperty(screen, 'height', { get: () => 1080, configurable: true });
        Object.defineProperty(screen, 'availWidth', { get: () => 1920, configurable: true });
        Object.defineProperty(screen, 'availHeight', { get: () => 1040, configurable: true });
        Object.defineProperty(screen, 'colorDepth', { get: () => 24, configurable: true });
        Object.defineProperty(screen, 'pixelDepth', { get: () => 24, configurable: true });
    } catch (_) {}

    // ===================================================================
    // (p) Remove CDP artifacts
    // ===================================================================
    try {
        for (const key of Object.keys(window)) { if (/^(cdc_|_cdc_)/.test(key)) { try { delete window[key]; } catch (_) {} } }
        for (const key of Object.keys(document)) { if (/^(\$cdc_|\$wdc_)/.test(key)) { try { delete document[key]; } catch (_) {} } }
    } catch (_) {}

    // ===================================================================
    // (q) Canvas fingerprint noise
    // ===================================================================
    try {
        const _origToDataURL = HTMLCanvasElement.prototype.toDataURL;
        const _origToBlob = HTMLCanvasElement.prototype.toBlob;
        const _origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
        const _noiseSeed = Math.floor(Math.random() * 256);
        const _addNoise = function(imageData) {
            const data = imageData.data;
            for (let i = 0; i < data.length; i += 4) {
                const noise = ((i * _noiseSeed) & 0xff) < 2 ? 1 : 0;
                data[i] = data[i] ^ noise;
                data[i + 1] = data[i + 1] ^ noise;
            }
            return imageData;
        };
        CanvasRenderingContext2D.prototype.getImageData = _markNative(function() {
            return _addNoise(_origGetImageData.apply(this, arguments));
        }, 'getImageData');
        HTMLCanvasElement.prototype.toDataURL = _markNative(function() {
            const ctx = this.getContext('2d');
            if (ctx) { try { ctx.fillStyle = 'rgba(0,0,0,0.004)'; ctx.fillRect(0, 0, 1, 1); } catch (_) {} }
            return _origToDataURL.apply(this, arguments);
        }, 'toDataURL');
        HTMLCanvasElement.prototype.toBlob = _markNative(function() {
            const ctx = this.getContext('2d');
            if (ctx) { try { ctx.fillStyle = 'rgba(0,0,0,0.004)'; ctx.fillRect(0, 0, 1, 1); } catch (_) {} }
            return _origToBlob.apply(this, arguments);
        }, 'toBlob');
    } catch (_) {}

    // ===================================================================
    // (r) AudioContext fingerprint noise
    // ===================================================================
    try {
        const _origGetFloat = AnalyserNode.prototype.getFloatFrequencyData;
        AnalyserNode.prototype.getFloatFrequencyData = _markNative(function(array) {
            _origGetFloat.call(this, array);
            for (let i = 0; i < array.length; i++) array[i] += (Math.random() - 0.5) * 0.001;
        }, 'getFloatFrequencyData');
        const _origGetChannel = AudioBuffer.prototype.getChannelData;
        AudioBuffer.prototype.getChannelData = _markNative(function(ch) {
            const data = _origGetChannel.call(this, ch);
            if (data.length < 480000) { for (let i = 0; i < data.length; i += 100) data[i] += (Math.random() - 0.5) * 1e-7; }
            return data;
        }, 'getChannelData');
    } catch (_) {}

    // ===================================================================
    // NEW (s) Stack trace sanitization — hide CDP injection artifacts
    // ===================================================================
    try {
        const _origPrepare = Error.prepareStackTrace;
        Error.prepareStackTrace = function(err, frames) {
            const filtered = frames.filter(f => {
                const fn = (f.getFileName && f.getFileName()) || '';
                return !fn.startsWith('pptr:') && !fn.includes('__puppeteer') && !fn.includes('__cdp') && !fn.startsWith('debugger:');
            });
            if (_origPrepare) return _origPrepare(err, filtered);
            return filtered.map(f => '    at ' + f.toString()).join('\n');
        };
    } catch (_) {}

    // ===================================================================
    // NEW (t) SpeechSynthesis voices — headless returns empty array
    // ===================================================================
    try {
        if (typeof speechSynthesis !== 'undefined') {
            const _voiceData = [
                { name: 'Alex', lang: 'en-US', localService: true, default: true, voiceURI: 'Alex' },
                { name: 'Samantha', lang: 'en-US', localService: true, default: false, voiceURI: 'Samantha' },
                { name: 'Daniel', lang: 'en-GB', localService: true, default: false, voiceURI: 'Daniel' },
                { name: 'Karen', lang: 'en-AU', localService: true, default: false, voiceURI: 'Karen' },
                { name: 'Moira', lang: 'en-IE', localService: true, default: false, voiceURI: 'Moira' },
                { name: 'Google US English', lang: 'en-US', localService: false, default: false, voiceURI: 'Google US English' },
                { name: 'Google UK English Female', lang: 'en-GB', localService: false, default: false, voiceURI: 'Google UK English Female' },
            ];
            const _voices = _voiceData.map(v => {
                const o = {};
                Object.defineProperties(o, {
                    name: { value: v.name, enumerable: true },
                    lang: { value: v.lang, enumerable: true },
                    localService: { value: v.localService, enumerable: true },
                    default: { value: v.default, enumerable: true },
                    voiceURI: { value: v.voiceURI, enumerable: true },
                });
                return o;
            });
            speechSynthesis.getVoices = _markNative(() => _voices, 'getVoices');
            const _origAddEL = speechSynthesis.addEventListener.bind(speechSynthesis);
            speechSynthesis.addEventListener = _markNative(function(type, cb, opts) {
                if (type === 'voiceschanged') { setTimeout(() => cb(), 10); return; }
                return _origAddEL(type, cb, opts);
            }, 'addEventListener');
        }
    } catch (_) {}

    // ===================================================================
    // NEW (u) CSS media query normalization — headless detection
    // ===================================================================
    try {
        const _origMM = window.matchMedia;
        window.matchMedia = _markNative(function(q) {
            const r = _origMM.call(window, q);
            const overrides = {
                '(pointer: fine)': true, '(hover: hover)': true,
                '(any-pointer: fine)': true, '(any-hover: hover)': true,
                '(prefers-reduced-motion: no-preference)': true,
            };
            if (q in overrides && !r.matches) {
                return {
                    matches: true, media: r.media, onchange: null,
                    addListener: r.addListener?.bind(r) || (() => {}),
                    removeListener: r.removeListener?.bind(r) || (() => {}),
                    addEventListener: r.addEventListener.bind(r),
                    removeEventListener: r.removeEventListener.bind(r),
                    dispatchEvent: r.dispatchEvent.bind(r),
                };
            }
            return r;
        }, 'matchMedia');
    } catch (_) {}

    // ===================================================================
    // NEW (v) navigator.userAgentData — ensure consistent high-entropy values
    // ===================================================================
    try {
        if (navigator.userAgentData) {
            const _origGetHEV = navigator.userAgentData.getHighEntropyValues.bind(navigator.userAgentData);
            navigator.userAgentData.getHighEntropyValues = _markNative(function(hints) {
                return _origGetHEV(hints).then(r => {
                    if (!r.fullVersionList || r.fullVersionList.length === 0) {
                        r.fullVersionList = [
                            { brand: 'Google Chrome', version: '135.0.7049.114' },
                            { brand: 'Chromium', version: '135.0.7049.114' },
                            { brand: 'Not-A.Brand', version: '99.0.0.0' },
                        ];
                    }
                    if (!r.fullVersion) r.fullVersion = '135.0.7049.114';
                    return r;
                });
            }, 'getHighEntropyValues');
        }
    } catch (_) {}

})();"##
}

/// Lightweight stealth script for CloakBrowser. CloakBrowser already handles
/// webdriver, plugins, languages, hardwareConcurrency, deviceMemory, platform,
/// WebGL, canvas, audio, and screen at the C++ level. We only patch things
/// the binary doesn't cover: window.chrome object, Permissions API,
/// Notification.permission, CDP artifact cleanup, and navigator.connection.
///
/// IMPORTANT: We do NOT touch navigator.webdriver, plugins, or WebGL here —
/// CloakBrowser's native patches are undetectable, our JS overrides are not.
pub fn get_cloakbrowser_stealth_script() -> &'static str {
    r##"(function() {
    'use strict';

    // ===================================================================
    // PHASE 0: Function.prototype.toString protection
    // ===================================================================
    const _nativeToString = Function.prototype.toString;
    const _patchedFns = new WeakMap();
    const _markNative = (fn, name) => { _patchedFns.set(fn, `function ${name}() { [native code] }`); return fn; };
    try {
        Function.prototype.toString = function() {
            if (_patchedFns.has(this)) return _patchedFns.get(this);
            return _nativeToString.call(this);
        };
        _patchedFns.set(Function.prototype.toString, 'function toString() { [native code] }');
    } catch (_) {}

    // (g) window.chrome — create a convincing chrome object
    try {
        if (!window.chrome) { window.chrome = {}; }
        if (!window.chrome.app) {
            window.chrome.app = {
                isInstalled: false,
                InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
                RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
                getDetails: _markNative(() => null, 'getDetails'),
                getIsInstalled: _markNative(() => false, 'getIsInstalled'),
            };
        }
        if (!window.chrome.runtime) {
            window.chrome.runtime = {
                OnInstalledReason: { CHROME_UPDATE: 'chrome_update', INSTALL: 'install', SHARED_MODULE_UPDATE: 'shared_module_update', UPDATE: 'update' },
                OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
                PlatformArch: { ARM: 'arm', ARM64: 'arm64', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
                PlatformNaclArch: { ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
                PlatformOs: { ANDROID: 'android', CROS: 'cros', FUCHSIA: 'fuchsia', LINUX: 'linux', MAC: 'mac', OPENBSD: 'openbsd', WIN: 'win' },
                RequestUpdateCheckStatus: { NO_UPDATE: 'no_update', THROTTLED: 'throttled', UPDATE_AVAILABLE: 'update_available' },
                connect: _markNative(() => { throw new TypeError('Error in invocation of runtime.connect'); }, 'connect'),
                sendMessage: _markNative(() => { throw new TypeError('Error in invocation of runtime.sendMessage'); }, 'sendMessage'),
                id: undefined,
            };
        }
        if (!window.chrome.loadTimes) {
            window.chrome.loadTimes = _markNative(function() {
                return { commitLoadTime: Date.now()/1000-0.5, connectionInfo:'h2', finishDocumentLoadTime: Date.now()/1000-0.1, finishLoadTime: Date.now()/1000-0.05, firstPaintAfterLoadTime:0, firstPaintTime: Date.now()/1000-0.3, navigationType:'Other', npnNegotiatedProtocol:'h2', requestTime: Date.now()/1000-1, startLoadTime: Date.now()/1000-0.8, wasAlternateProtocolAvailable:false, wasFetchedViaSpdy:true, wasNpnNegotiated:true };
            }, 'loadTimes');
        }
        if (!window.chrome.csi) {
            window.chrome.csi = _markNative(function() {
                return { onloadT: Date.now(), pageT: Date.now()-performance.timing.navigationStart, startE: performance.timing.navigationStart, tran: 15 };
            }, 'csi');
        }
    } catch (_) {}

    // (h) Permissions API — return "prompt" for notification
    try {
        const oq = Permissions.prototype.query;
        Permissions.prototype.query = _markNative(function(p) {
            if (p && p.name === 'notifications') return Promise.resolve({ state: 'prompt', onchange: null });
            return oq.call(this, p);
        }, 'query');
    } catch (_) {}

    // (n) Notification.permission — return "default"
    try {
        Object.defineProperty(Notification, 'permission', { get: () => 'default', configurable: true });
    } catch (_) {}

    // (p) Remove CDP artifacts
    try {
        for (const key of Object.keys(window)) { if (/^(cdc_|_cdc_)/.test(key)) { try { delete window[key]; } catch (_) {} } }
        for (const key of Object.keys(document)) { if (/^(\$cdc_|\$wdc_)/.test(key)) { try { delete document[key]; } catch (_) {} } }
    } catch (_) {}

    // (m) navigator.connection — mock if missing
    try {
        if (!navigator.connection) {
            const ci = { rtt:50, downlink:10, effectiveType:'4g', saveData:false, onchange:null,
                addEventListener:_markNative(()=>{}, 'addEventListener'),
                removeEventListener:_markNative(()=>{}, 'removeEventListener'),
                dispatchEvent:_markNative(()=>true, 'dispatchEvent') };
            Object.defineProperty(navigator, 'connection', { get: () => ci, configurable: true });
        }
    } catch (_) {}

    // (s) Stack trace sanitization — hide CDP injection artifacts
    try {
        const _origPrepare = Error.prepareStackTrace;
        Error.prepareStackTrace = function(err, frames) {
            const filtered = frames.filter(f => {
                const fn = (f.getFileName && f.getFileName()) || '';
                return !fn.startsWith('pptr:') && !fn.includes('__puppeteer') && !fn.includes('__cdp') && !fn.startsWith('debugger:');
            });
            if (_origPrepare) return _origPrepare(err, filtered);
            return filtered.map(f => '    at ' + f.toString()).join('\n');
        };
    } catch (_) {}

    // (t) SpeechSynthesis voices — headless returns empty array
    try {
        if (typeof speechSynthesis !== 'undefined') {
            const _voiceData = [
                { name: 'Alex', lang: 'en-US', localService: true, default: true, voiceURI: 'Alex' },
                { name: 'Samantha', lang: 'en-US', localService: true, default: false, voiceURI: 'Samantha' },
                { name: 'Daniel', lang: 'en-GB', localService: true, default: false, voiceURI: 'Daniel' },
                { name: 'Karen', lang: 'en-AU', localService: true, default: false, voiceURI: 'Karen' },
                { name: 'Moira', lang: 'en-IE', localService: true, default: false, voiceURI: 'Moira' },
                { name: 'Google US English', lang: 'en-US', localService: false, default: false, voiceURI: 'Google US English' },
                { name: 'Google UK English Female', lang: 'en-GB', localService: false, default: false, voiceURI: 'Google UK English Female' },
            ];
            const _voices = _voiceData.map(v => {
                const o = {};
                Object.defineProperties(o, {
                    name: { value: v.name, enumerable: true },
                    lang: { value: v.lang, enumerable: true },
                    localService: { value: v.localService, enumerable: true },
                    default: { value: v.default, enumerable: true },
                    voiceURI: { value: v.voiceURI, enumerable: true },
                });
                return o;
            });
            speechSynthesis.getVoices = _markNative(() => _voices, 'getVoices');
            const _origAddEL = speechSynthesis.addEventListener.bind(speechSynthesis);
            speechSynthesis.addEventListener = _markNative(function(type, cb, opts) {
                if (type === 'voiceschanged') { setTimeout(() => cb(), 10); return; }
                return _origAddEL(type, cb, opts);
            }, 'addEventListener');
        }
    } catch (_) {}

    // (u) CSS media query normalization — headless detection
    try {
        const _origMM = window.matchMedia;
        window.matchMedia = _markNative(function(q) {
            const r = _origMM.call(window, q);
            const overrides = {
                '(pointer: fine)': true, '(hover: hover)': true,
                '(any-pointer: fine)': true, '(any-hover: hover)': true,
                '(prefers-reduced-motion: no-preference)': true,
            };
            if (q in overrides && !r.matches) {
                return {
                    matches: true, media: r.media, onchange: null,
                    addListener: r.addListener?.bind(r) || (() => {}),
                    removeListener: r.removeListener?.bind(r) || (() => {}),
                    addEventListener: r.addEventListener.bind(r),
                    removeEventListener: r.removeEventListener.bind(r),
                    dispatchEvent: r.dispatchEvent.bind(r),
                };
            }
            return r;
        }, 'matchMedia');
    } catch (_) {}

    // (v) navigator.userAgentData — ensure consistent high-entropy values
    try {
        if (navigator.userAgentData) {
            const _origGetHEV = navigator.userAgentData.getHighEntropyValues.bind(navigator.userAgentData);
            navigator.userAgentData.getHighEntropyValues = _markNative(function(hints) {
                return _origGetHEV(hints).then(r => {
                    if (!r.fullVersionList || r.fullVersionList.length === 0) {
                        r.fullVersionList = [
                            { brand: 'Google Chrome', version: '135.0.7049.114' },
                            { brand: 'Chromium', version: '135.0.7049.114' },
                            { brand: 'Not-A.Brand', version: '99.0.0.0' },
                        ];
                    }
                    if (!r.fullVersion) r.fullVersion = '135.0.7049.114';
                    return r;
                });
            }, 'getHighEntropyValues');
        }
    } catch (_) {}

})();"##
}

/// Returns a realistic, recent Chrome user agent string suitable for macOS.
pub fn get_default_user_agent() -> &'static str {
    CHROME_UA
}

/// Returns default HTTP headers that a real Chrome browser would send.
/// Used only when running stock Chrome (not CloakBrowser).
pub fn get_stealth_headers() -> Vec<(&'static str, &'static str)> {
    // IMPORTANT: Do NOT include Sec-Fetch-* headers here.
    // Network.setExtraHTTPHeaders applies to ALL requests (scripts, images, etc.).
    // Sec-Fetch-Dest/Mode/Site vary per request type — Chrome sets them correctly
    // on its own. Overriding them breaks subresource loading (e.g. Cloudflare
    // Turnstile scripts get rejected when they arrive with Sec-Fetch-Dest: document).
    vec![
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Sec-Ch-Ua", "\"Google Chrome\";v=\"135\", \"Chromium\";v=\"135\", \"Not-A.Brand\";v=\"99\""),
        ("Sec-Ch-Ua-Full-Version-List", "\"Google Chrome\";v=\"135.0.7049.114\", \"Chromium\";v=\"135.0.7049.114\", \"Not-A.Brand\";v=\"99.0.0.0\""),
        ("Sec-Ch-Ua-Mobile", "?0"),
        ("Sec-Ch-Ua-Platform", "\"macOS\""),
    ]
}

/// Minimal headers for CloakBrowser — only things the binary doesn't set.
/// Does NOT include Sec-Ch-Ua (CloakBrowser sets its own matching the actual version).
pub fn get_cloakbrowser_headers() -> Vec<(&'static str, &'static str)> {
    // Same principle: no Sec-Fetch-* overrides (let Chrome handle per-request).
    vec![("Accept-Language", "en-US,en;q=0.9")]
}

/// Returns Client Hints metadata for Emulation.setUserAgentOverride.
/// This populates navigator.userAgentData.getHighEntropyValues() correctly.
pub fn get_user_agent_metadata() -> &'static str {
    r#"{
        "brands": [
            {"brand": "Google Chrome", "version": "135"},
            {"brand": "Chromium", "version": "135"},
            {"brand": "Not-A.Brand", "version": "99"}
        ],
        "fullVersionList": [
            {"brand": "Google Chrome", "version": "135.0.7049.114"},
            {"brand": "Chromium", "version": "135.0.7049.114"},
            {"brand": "Not-A.Brand", "version": "99.0.0.0"}
        ],
        "fullVersion": "135.0.7049.114",
        "platform": "macOS",
        "platformVersion": "15.3.0",
        "architecture": "arm",
        "model": "",
        "mobile": false,
        "bitness": "64",
        "wow64": false
    }"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stealth_args_are_non_empty() {
        let args = get_stealth_chrome_args();
        assert!(!args.is_empty());
        assert!(args.iter().all(|a| a.starts_with("--")));
    }

    #[test]
    fn stealth_script_is_an_iife() {
        let script = get_stealth_script();
        assert!(script.starts_with("(function()"));
        assert!(script.trim_end().ends_with("();"));
    }

    #[test]
    fn user_agent_looks_valid() {
        let ua = get_default_user_agent();
        assert!(ua.contains("Chrome/135"));
        assert!(ua.contains("Mozilla/5.0"));
    }

    #[test]
    fn stealth_headers_are_non_empty() {
        let headers = get_stealth_headers();
        assert!(!headers.is_empty());
        assert!(headers.iter().any(|(k, _)| *k == "Accept-Language"));
    }
}
