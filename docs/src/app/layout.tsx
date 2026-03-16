import type { Metadata } from "next";
import { Inter, Geist_Mono } from "next/font/google";
import "./globals.css";

const inter = Inter({
  variable: "--font-inter",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  metadataBase: new URL("https://silicon-browser.unlikefraction.com"),
  title: {
    default: "silicon-browser | The Most Reliable Browser for Your AI Agent",
    template: "%s | silicon-browser",
  },
  description:
    "Stealth-first, terminal-native headless browser CLI for AI agents. CloakBrowser + stealth evasions. Profiles with pinned fingerprints. Push/clone over HTTP.",
  openGraph: {
    type: "website",
    locale: "en_US",
    url: "https://silicon-browser.unlikefraction.com",
    siteName: "silicon-browser",
    title: "silicon-browser | The Most Reliable Browser for Your AI Agent",
    description:
      "Stealth-first, terminal-native headless browser CLI for AI agents. CloakBrowser + stealth evasions. Profiles with pinned fingerprints. Push/clone over HTTP.",
    images: [
      { url: "/og", width: 1200, height: 630, alt: "silicon-browser" },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "silicon-browser | The Most Reliable Browser for Your AI Agent",
    description:
      "Stealth-first, terminal-native headless browser CLI for AI agents. CloakBrowser + stealth evasions. Profiles with pinned fingerprints. Push/clone over HTTP.",
    images: ["/og"],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${inter.variable} ${geistMono.variable}`}>
        {children}
      </body>
    </html>
  );
}
