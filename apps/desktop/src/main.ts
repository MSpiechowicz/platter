import { mount } from 'svelte';
// Inter ships with the app (SIL Open Font License), so text looks and centres the same on every machine.
import '@fontsource-variable/inter/index.css';
import './app.css';
import App from './App.svelte';
import { trackInputModality } from './lib/focus';
import { hooks } from './lib/hooks';
import { settings } from './lib/state/settings.svelte';
import { applyThemeChoice } from './lib/state/theme.svelte';

// ?static: no animation from the very first frame.
if (hooks.static) {
  document.documentElement.dataset.motion = 'reduce';
}

// The saved theme (or the hook's, which is not saved) from the very first frame, so a light theme does not start dark.
applyThemeChoice(hooks.theme ?? settings.theme);

trackInputModality();

const target = document.getElementById('app');

if (!target) {
  throw new Error('Missing #app element');
}

mount(App, { target });
