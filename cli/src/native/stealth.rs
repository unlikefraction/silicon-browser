// stealth.rs - Anti-detection and stealth evasion logic for silicon-browser.
//
// This module provides Chrome launch arguments, JavaScript injection scripts,
// user-agent strings, and HTTP headers designed to make automated Chrome
// instances appear indistinguishable from a regular human-operated browser.
//
// The stealth script is injected via Page.addScriptToEvaluateOnNewDocument so
// that every frame (including iframes) receives the patches before any page
// code can observe the default (detectable) values.

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
        "--disable-features=AutomationControlled,AcceptCHFrame,MediaRouter,OptimizationHints".to_string(),
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

    // -----------------------------------------------------------------------
    // (a) navigator.webdriver  --  must be false, defined as a property so
    //     that naive checks like `navigator.webdriver` AND
    //     `Object.getOwnPropertyDescriptor(navigator, 'webdriver')` both pass.
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(navigator, 'webdriver', {
            get: () => false,
            configurable: true,
        });
        // Also patch the prototype in case code walks the proto chain.
        Object.defineProperty(Navigator.prototype, 'webdriver', {
            get: () => false,
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (b) navigator.plugins  --  mock five realistic plugins with a proper
    //     PluginArray interface.
    // -----------------------------------------------------------------------
    try {
        const pluginData = [
            { name: 'Chrome PDF Plugin',       filename: 'internal-pdf-viewer',       description: 'Portable Document Format' },
            { name: 'Chrome PDF Viewer',        filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
            { name: 'Native Client',            filename: 'internal-nacl-plugin',      description: '' },
            { name: 'Chromium PDF Plugin',      filename: 'internal-pdf-viewer',       description: 'Portable Document Format' },
            { name: 'Chromium PDF Viewer',      filename: 'internal-pdf-viewer',       description: 'Portable Document Format' },
        ];

        const makeMimeType = (p) => ({
            type: 'application/pdf',
            suffixes: 'pdf',
            description: 'Portable Document Format',
            enabledPlugin: p,
        });

        const plugins = pluginData.map((d) => {
            const p = Object.create(Plugin.prototype);
            Object.defineProperties(p, {
                name:        { value: d.name,        enumerable: true },
                filename:    { value: d.filename,    enumerable: true },
                description: { value: d.description, enumerable: true },
                length:      { value: 1,             enumerable: true },
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
        pluginArray.item = (i) => plugins[i] || null;
        pluginArray.namedItem = (n) => plugins.find((p) => p.name === n) || null;
        pluginArray.refresh = () => {};

        Object.defineProperty(navigator, 'plugins', {
            get: () => pluginArray,
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (c) navigator.languages
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(navigator, 'languages', {
            get: () => ['en-US', 'en'],
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (d) navigator.hardwareConcurrency  --  report 8 cores
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(navigator, 'hardwareConcurrency', {
            get: () => 8,
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (e) navigator.deviceMemory  --  report 8 GB
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(navigator, 'deviceMemory', {
            get: () => 8,
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (f) navigator.platform  --  set to correct platform value
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(navigator, 'platform', {
            get: () => {
                const ua = navigator.userAgent || '';
                if (ua.includes('Mac'))   return 'MacIntel';
                if (ua.includes('Win'))   return 'Win32';
                if (ua.includes('Linux')) return 'Linux x86_64';
                return 'MacIntel'; // default fallback
            },
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (g) window.chrome  --  create a convincing chrome object
    // -----------------------------------------------------------------------
    try {
        if (!window.chrome) {
            window.chrome = {};
        }
        window.chrome.app = {
            isInstalled: false,
            InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
            RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
            getDetails: () => null,
            getIsInstalled: () => false,
        };
        window.chrome.runtime = {
            OnInstalledReason: {
                CHROME_UPDATE: 'chrome_update',
                INSTALL: 'install',
                SHARED_MODULE_UPDATE: 'shared_module_update',
                UPDATE: 'update',
            },
            OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
            PlatformArch: {
                ARM: 'arm', ARM64: 'arm64', MIPS: 'mips', MIPS64: 'mips64',
                X86_32: 'x86-32', X86_64: 'x86-64',
            },
            PlatformNaclArch: {
                ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64',
                X86_32: 'x86-32', X86_64: 'x86-64',
            },
            PlatformOs: {
                ANDROID: 'android', CROS: 'cros', FUCHSIA: 'fuchsia',
                LINUX: 'linux', MAC: 'mac', OPENBSD: 'openbsd', WIN: 'win',
            },
            RequestUpdateCheckStatus: {
                NO_UPDATE: 'no_update', THROTTLED: 'throttled', UPDATE_AVAILABLE: 'update_available',
            },
            connect: () => { throw new TypeError('Error in invocation of runtime.connect'); },
            sendMessage: () => { throw new TypeError('Error in invocation of runtime.sendMessage'); },
            id: undefined,
        };
        window.chrome.loadTimes = function() {
            return {
                commitLoadTime: Date.now() / 1000 - 0.5,
                connectionInfo: 'h2',
                finishDocumentLoadTime: Date.now() / 1000 - 0.1,
                finishLoadTime: Date.now() / 1000 - 0.05,
                firstPaintAfterLoadTime: 0,
                firstPaintTime: Date.now() / 1000 - 0.3,
                navigationType: 'Other',
                npnNegotiatedProtocol: 'h2',
                requestTime: Date.now() / 1000 - 1,
                startLoadTime: Date.now() / 1000 - 0.8,
                wasAlternateProtocolAvailable: false,
                wasFetchedViaSpdy: true,
                wasNpnNegotiated: true,
            };
        };
        window.chrome.csi = function() {
            return {
                onloadT: Date.now(),
                pageT: Date.now() - performance.timing.navigationStart,
                startE: performance.timing.navigationStart,
                tran: 15,
            };
        };
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (h) Permissions API  --  return "prompt" for notification permission
    // -----------------------------------------------------------------------
    try {
        const originalQuery = Permissions.prototype.query;
        Permissions.prototype.query = function(parameters) {
            if (parameters && parameters.name === 'notifications') {
                return Promise.resolve({ state: 'prompt', onchange: null });
            }
            return originalQuery.call(this, parameters);
        };
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (i) WebGL vendor / renderer  --  mask SwiftShader strings
    // -----------------------------------------------------------------------
    try {
        const getParameterProto = WebGLRenderingContext.prototype.getParameter;
        const UNMASKED_VENDOR_WEBGL  = 0x9245;
        const UNMASKED_RENDERER_WEBGL = 0x9246;

        WebGLRenderingContext.prototype.getParameter = function(parameter) {
            if (parameter === UNMASKED_VENDOR_WEBGL)  return 'Google Inc. (NVIDIA)';
            if (parameter === UNMASKED_RENDERER_WEBGL) return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1080 Ti Direct3D11 vs_5_0 ps_5_0, D3D11)';
            return getParameterProto.call(this, parameter);
        };

        // Patch WebGL2 as well.
        if (typeof WebGL2RenderingContext !== 'undefined') {
            const getParameterProto2 = WebGL2RenderingContext.prototype.getParameter;
            WebGL2RenderingContext.prototype.getParameter = function(parameter) {
                if (parameter === UNMASKED_VENDOR_WEBGL)  return 'Google Inc. (NVIDIA)';
                if (parameter === UNMASKED_RENDERER_WEBGL) return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1080 Ti Direct3D11 vs_5_0 ps_5_0, D3D11)';
                return getParameterProto2.call(this, parameter);
            };
        }
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (j) iframe contentWindow  --  proxy cross-origin iframes to prevent
    //     detection via toString / object identity checks
    // -----------------------------------------------------------------------
    try {
        const originalContentWindow = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
        if (originalContentWindow && originalContentWindow.get) {
            Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
                get: function() {
                    const win = originalContentWindow.get.call(this);
                    if (!win) return win;
                    try {
                        // Accessing .document on a cross-origin iframe throws; if it does
                        // NOT throw we are same-origin and can return normally.
                        void win.document;
                        return win;
                    } catch (_) {
                        // Cross-origin -- return a Proxy so detection scripts that
                        // compare typeof results get consistent answers.
                        return new Proxy(win, {
                            get: function(target, prop) {
                                try { return target[prop]; } catch (_) { return undefined; }
                            },
                        });
                    }
                },
                configurable: true,
            });
        }
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (k) console.debug  --  prevent detection via console tricks
    // -----------------------------------------------------------------------
    try {
        const nativeToString = Function.prototype.toString;
        const nativeLog   = console.log;
        const nativeDebug = console.debug;
        // Some detectors override console.debug and check if the native code
        // string survives; keep the originals intact.
        Object.defineProperty(console, 'debug', {
            value: nativeDebug,
            writable: false,
            configurable: false,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (l) window.outerWidth / outerHeight  --  non-zero, offset values
    // -----------------------------------------------------------------------
    try {
        if (window.outerWidth === 0) {
            Object.defineProperty(window, 'outerWidth', {
                get: () => window.innerWidth + 15,
                configurable: true,
            });
        }
        if (window.outerHeight === 0) {
            Object.defineProperty(window, 'outerHeight', {
                get: () => window.innerHeight + 85,
                configurable: true,
            });
        }
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (m) navigator.connection  --  mock NetworkInformation
    // -----------------------------------------------------------------------
    try {
        if (!navigator.connection) {
            const connectionInfo = {
                rtt: 50,
                downlink: 10,
                effectiveType: '4g',
                saveData: false,
                onchange: null,
            };
            // Provide addEventListener / removeEventListener stubs expected on
            // EventTarget-like objects.
            connectionInfo.addEventListener    = () => {};
            connectionInfo.removeEventListener = () => {};
            connectionInfo.dispatchEvent       = () => true;

            Object.defineProperty(navigator, 'connection', {
                get: () => connectionInfo,
                configurable: true,
            });
        }
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (n) Notification.permission  --  return "default"
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(Notification, 'permission', {
            get: () => 'default',
            configurable: true,
        });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (o) window.screen  --  ensure consistent size and colorDepth
    // -----------------------------------------------------------------------
    try {
        Object.defineProperty(screen, 'width',      { get: () => 1920, configurable: true });
        Object.defineProperty(screen, 'height',     { get: () => 1080, configurable: true });
        Object.defineProperty(screen, 'availWidth',  { get: () => 1920, configurable: true });
        Object.defineProperty(screen, 'availHeight', { get: () => 1040, configurable: true }); // taskbar offset
        Object.defineProperty(screen, 'colorDepth',  { get: () => 24,   configurable: true });
        Object.defineProperty(screen, 'pixelDepth',  { get: () => 24,   configurable: true });
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (p) Remove CDP artifacts  --  delete cdc_* properties
    // -----------------------------------------------------------------------
    try {
        for (const key of Object.keys(window)) {
            if (/^cdc_/.test(key) || /^_cdc_/.test(key)) {
                try { delete window[key]; } catch (_) {}
            }
        }
        // Also remove $cdc_ from document.
        for (const key of Object.keys(document)) {
            if (/^(\$cdc_|\$wdc_)/.test(key)) {
                try { delete document[key]; } catch (_) {}
            }
        }
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (q) Canvas fingerprint noise  --  add subtle random noise
    // -----------------------------------------------------------------------
    try {
        const originalToDataURL = HTMLCanvasElement.prototype.toDataURL;
        const originalToBlob    = HTMLCanvasElement.prototype.toBlob;
        const originalGetImageData = CanvasRenderingContext2D.prototype.getImageData;

        // Small per-session seed so noise is consistent within the same page
        // load but differs between sessions.
        const noiseSeed = Math.floor(Math.random() * 256);

        const addNoise = function(imageData) {
            const data = imageData.data;
            for (let i = 0; i < data.length; i += 4) {
                // XOR a small deterministic-ish value (0 or 1) into the least
                // significant bit.  This is invisible to the eye but changes
                // the hash.
                const noise = ((i * noiseSeed) & 0xff) < 2 ? 1 : 0;
                data[i]     = data[i] ^ noise;     // R
                data[i + 1] = data[i + 1] ^ noise; // G
            }
            return imageData;
        };

        CanvasRenderingContext2D.prototype.getImageData = function() {
            const imageData = originalGetImageData.apply(this, arguments);
            return addNoise(imageData);
        };

        HTMLCanvasElement.prototype.toDataURL = function() {
            const ctx = this.getContext('2d');
            if (ctx) {
                // Force a tiny invisible modification so toDataURL output shifts.
                try {
                    ctx.fillStyle = 'rgba(0,0,0,0.004)';
                    ctx.fillRect(0, 0, 1, 1);
                } catch (_) {}
            }
            return originalToDataURL.apply(this, arguments);
        };

        HTMLCanvasElement.prototype.toBlob = function() {
            const ctx = this.getContext('2d');
            if (ctx) {
                try {
                    ctx.fillStyle = 'rgba(0,0,0,0.004)';
                    ctx.fillRect(0, 0, 1, 1);
                } catch (_) {}
            }
            return originalToBlob.apply(this, arguments);
        };
    } catch (_) {}

    // -----------------------------------------------------------------------
    // (r) AudioContext fingerprint noise
    // -----------------------------------------------------------------------
    try {
        const origCreateOscillator = AudioContext.prototype.createOscillator;
        const origCreateDynamicsCompressor = AudioContext.prototype.createDynamicsCompressor;
        const origGetFloatFrequencyData = AnalyserNode.prototype.getFloatFrequencyData;

        AnalyserNode.prototype.getFloatFrequencyData = function(array) {
            origGetFloatFrequencyData.call(this, array);
            // Add very small noise to each sample.
            for (let i = 0; i < array.length; i++) {
                array[i] += (Math.random() - 0.5) * 0.001;
            }
        };

        // Patch getChannelData on AudioBuffer to inject micro-noise.
        const origGetChannelData = AudioBuffer.prototype.getChannelData;
        AudioBuffer.prototype.getChannelData = function(channel) {
            const data = origGetChannelData.call(this, channel);
            // Only inject noise if the buffer is short (fingerprinting buffers
            // are typically < 10 seconds).
            if (data.length < 480000) {
                for (let i = 0; i < data.length; i += 100) {
                    data[i] += (Math.random() - 0.5) * 1e-7;
                }
            }
            return data;
        };
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

    // (g) window.chrome — create a convincing chrome object
    try {
        if (!window.chrome) { window.chrome = {}; }
        if (!window.chrome.app) {
            window.chrome.app = {
                isInstalled: false,
                InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
                RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
                getDetails: () => null, getIsInstalled: () => false,
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
                connect: () => { throw new TypeError('Error in invocation of runtime.connect'); },
                sendMessage: () => { throw new TypeError('Error in invocation of runtime.sendMessage'); },
                id: undefined,
            };
        }
        if (!window.chrome.loadTimes) {
            window.chrome.loadTimes = function() {
                return { commitLoadTime: Date.now()/1000-0.5, connectionInfo:'h2', finishDocumentLoadTime: Date.now()/1000-0.1, finishLoadTime: Date.now()/1000-0.05, firstPaintAfterLoadTime:0, firstPaintTime: Date.now()/1000-0.3, navigationType:'Other', npnNegotiatedProtocol:'h2', requestTime: Date.now()/1000-1, startLoadTime: Date.now()/1000-0.8, wasAlternateProtocolAvailable:false, wasFetchedViaSpdy:true, wasNpnNegotiated:true };
            };
        }
        if (!window.chrome.csi) {
            window.chrome.csi = function() {
                return { onloadT: Date.now(), pageT: Date.now()-performance.timing.navigationStart, startE: performance.timing.navigationStart, tran: 15 };
            };
        }
    } catch (_) {}

    // (h) Permissions API — return "prompt" for notification
    try {
        const oq = Permissions.prototype.query;
        Permissions.prototype.query = function(p) {
            if (p && p.name === 'notifications') return Promise.resolve({ state: 'prompt', onchange: null });
            return oq.call(this, p);
        };
    } catch (_) {}

    // (n) Notification.permission — return "default"
    try {
        Object.defineProperty(Notification, 'permission', { get: () => 'default', configurable: true });
    } catch (_) {}

    // (p) Remove CDP artifacts
    try {
        for (const key of Object.keys(window)) {
            if (/^cdc_/.test(key) || /^_cdc_/.test(key)) { try { delete window[key]; } catch (_) {} }
        }
        for (const key of Object.keys(document)) {
            if (/^(\$cdc_|\$wdc_)/.test(key)) { try { delete document[key]; } catch (_) {} }
        }
    } catch (_) {}

    // (m) navigator.connection — mock if missing
    try {
        if (!navigator.connection) {
            const ci = { rtt:50, downlink:10, effectiveType:'4g', saveData:false, onchange:null, addEventListener:()=>{}, removeEventListener:()=>{}, dispatchEvent:()=>true };
            Object.defineProperty(navigator, 'connection', { get: () => ci, configurable: true });
        }
    } catch (_) {}

})();"##
}

/// Returns a realistic, recent Chrome user agent string suitable for macOS.
pub fn get_default_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
}

/// Returns default HTTP headers that a real Chrome browser would send.
/// Used only when running stock Chrome (not CloakBrowser).
pub fn get_stealth_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"),
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Accept-Encoding", "gzip, deflate, br, zstd"),
        ("Sec-Ch-Ua", "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\""),
        ("Sec-Ch-Ua-Mobile", "?0"),
        ("Sec-Ch-Ua-Platform", "\"macOS\""),
        ("Sec-Fetch-Dest", "document"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Site", "none"),
        ("Sec-Fetch-User", "?1"),
        ("Upgrade-Insecure-Requests", "1"),
    ]
}

/// Minimal headers for CloakBrowser — only things the binary doesn't set.
/// Does NOT include Sec-Ch-Ua (CloakBrowser sets its own matching the actual version).
pub fn get_cloakbrowser_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"),
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Accept-Encoding", "gzip, deflate, br, zstd"),
        ("Sec-Fetch-Dest", "document"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Site", "none"),
        ("Sec-Fetch-User", "?1"),
        ("Upgrade-Insecure-Requests", "1"),
    ]
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
        assert!(ua.contains("Chrome/"));
        assert!(ua.contains("Mozilla/5.0"));
    }

    #[test]
    fn stealth_headers_are_non_empty() {
        let headers = get_stealth_headers();
        assert!(!headers.is_empty());
        assert!(headers.iter().any(|(k, _)| *k == "Accept"));
    }
}
