import { mount } from 'svelte';
// Inter ships with the app (SIL Open Font License), so text looks and centres the same on every machine.
import '@fontsource-variable/inter/index.css';
import './app.css';
import App from './App.svelte';
import { trackInputModality } from './lib/focus';
import { hooks } from './lib/hooks';

// ?static: no animation from the very first frame.
if (hooks.static) {
  document.documentElement.dataset.motion = 'reduce';
}

trackInputModality();

const target = document.getElementById('app');

if (!target) {
  throw new Error('Missing #app element');
}

mount(App, { target });
