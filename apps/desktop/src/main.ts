import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { openExternalUrl } from './lib/api';

// Interceptador global para enlaces externos: previene que WebView2 navegue fuera de ROMSIDIAN
document.addEventListener(
  'click',
  (event: MouseEvent) => {
    const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a');
    if (!anchor) return;
    const href = anchor.getAttribute('href') || anchor.href;
    if (
      href &&
      (href.startsWith('http://') ||
        href.startsWith('https://') ||
        href.startsWith('mailto:'))
    ) {
      event.preventDefault();
      event.stopPropagation();
      void openExternalUrl(href);
    }
  },
  true,
);

const app = mount(App, {
  target: document.getElementById('app')!,
});

export default app;
