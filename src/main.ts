import '@fontsource-variable/archivo/wdth.css';
import '@fontsource-variable/inter/index.css';
import './base.css';
import './skins/paper.css';
import './skins/glass.css';
import './skins/fluent.css';

import { invoke } from '@tauri-apps/api/core';

import { panelApp } from './panelApp';
import { settingsApp } from './settingsApp';
import type { Bootstrap } from './types';

// One page serves both windows; the settings window is opened with ?settings.
const boot = await invoke<Bootstrap>('bootstrap');
if (new URLSearchParams(location.search).has('settings')) {
  await settingsApp(boot);
} else {
  await panelApp(boot);
}
