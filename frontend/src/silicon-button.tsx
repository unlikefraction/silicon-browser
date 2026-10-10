import { createEffect, onCleanup, onMount } from 'solid-js';
import { createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { Button, type ButtonVariant } from './components/silicon-ui/button/button';

/** Preserve Solid's session lifecycle and render the actual React registry control. */
export function SiliconButton(props: {
  children: string;
  type?: 'button' | 'submit' | 'reset';
  variant?: ButtonVariant;
  disabled?: boolean;
  onClick?: () => void;
  'aria-expanded'?: boolean;
}) {
  let host!: HTMLSpanElement;
  let root: Root | undefined;
  onMount(() => {
    root = createRoot(host);
    createEffect(() => root!.render(createElement(Button, {
      type: props.type ?? 'submit',
      variant: props.variant ?? 'secondary',
      disabled: props.disabled,
      onClick: props.onClick,
      'aria-expanded': props['aria-expanded'],
    }, props.children)));
  });
  onCleanup(() => root?.unmount());
  return <span class="silicon-control" ref={host}/>;
}
