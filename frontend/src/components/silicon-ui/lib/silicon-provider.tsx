"use client";

/**
 * Lets an app swap the link and image elements Silicon UI components render.
 *
 * Without a provider, Silicon UI renders a plain `<a>` and `<img>`, so components work in any React app (Vite, Remix, Astro,
 * plain React). Wrap the app once to use your framework's or your own components instead:
 *
 *   import Link from "next/link";
 *   import Image from "next/image";
 *   import { SiliconProvider } from "@/components/silicon-ui/lib/silicon-provider";
 *
 *   <SiliconProvider link={Link} image={Image}>{children}</SiliconProvider>
 *
 * Any component that accepts these props works, such as a locale-aware Link from next-intl or a React Router Link
 * wrapped to map `href` to `to`.
 */
import { createContext, createElement, useContext, useMemo, type AnchorHTMLAttributes, type ComponentType, type ImgHTMLAttributes, type ReactNode, type Ref } from "react";

/** The props Silicon UI passes to a link: an href plus ordinary anchor attributes. */
export interface SiliconLinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  href: string;
  ref?: Ref<HTMLAnchorElement>;
}

/**
 * The props Silicon UI passes to an image. A subset of next/image's props, so next/image can be passed as is: either `fill`
 * (cover the positioned parent) or `width` and `height`, plus `sizes` and `priority` as hints.
 */
export interface SiliconImageProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, "src" | "alt" | "width" | "height" | "loading" | "srcSet"> {
  src: string;
  alt: string;
  width?: number;
  height?: number;
  /** Fills the nearest positioned parent, like next/image's `fill`. */
  fill?: boolean;
  /** Loads the image eagerly with high priority, like next/image's `priority`. */
  priority?: boolean;
  loading?: "eager" | "lazy";
  ref?: Ref<HTMLImageElement>;
}

export type SiliconLinkComponent = ComponentType<SiliconLinkProps>;
export type SiliconImageComponent = ComponentType<SiliconImageProps>;

/** The default link: a plain anchor. */
export function SiliconAnchor(props: SiliconLinkProps) {
  return <a {...props} />;
}

const fillStyle = { position: "absolute", inset: 0, width: "100%", height: "100%" } as const;

/** The default image: a plain, lazily decoded `<img>` that understands `fill` and `priority`. */
export function SiliconImg({ alt, fill, priority, loading, style, ...props }: SiliconImageProps) {
  // eslint-disable-next-line @next/next/no-img-element -- the framework-free default; pass next/image through SiliconProvider instead.
  return <img alt={alt} decoding="async" {...props} loading={loading ?? (priority ? "eager" : "lazy")} fetchPriority={priority ? "high" : props.fetchPriority} style={fill ? { ...fillStyle, ...style } : style} />;
}

interface SiliconComponents {
  link: SiliconLinkComponent;
  image: SiliconImageComponent;
}

const SiliconContext = createContext<SiliconComponents>({ link: SiliconAnchor, image: SiliconImg });

export interface SiliconProviderProps {
  /** The link component Silicon UI renders for navigation, such as next/link or your own wrapped Link. Defaults to `<a>`. */
  link?: SiliconLinkComponent;
  /** The image component Silicon UI renders for photos, such as next/image. Defaults to `<img>`. */
  image?: SiliconImageComponent;
  children: ReactNode;
}

/** Sets the link and image components for every Silicon UI component inside it. Nested providers inherit what they do not set. */
export function SiliconProvider({ link, image, children }: SiliconProviderProps) {
  const parent = useContext(SiliconContext);
  const value = useMemo(() => ({ link: link ?? parent.link, image: image ?? parent.image }), [link, image, parent]);
  return <SiliconContext.Provider value={value}>{children}</SiliconContext.Provider>;
}

/** The link component from the nearest SiliconProvider, or a plain anchor. */
export const useArcLink = () => useContext(SiliconContext).link;
/** The image component from the nearest SiliconProvider, or a plain img. */
export const useArcImage = () => useContext(SiliconContext).image;

/** What Silicon UI components render for a link: the provider's link component, or a plain anchor. */
export function SiliconLink(props: SiliconLinkProps) {
  return createElement(useArcLink(), props);
}

/** What Silicon UI components render for an image: the provider's image component, or a plain img. */
export function SiliconImage(props: SiliconImageProps) {
  return createElement(useArcImage(), props);
}
