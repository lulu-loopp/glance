import '@fontsource-variable/archivo/wdth.css';
import '@fontsource-variable/inter/index.css';
import './base.css';
import './skins/paper.css';
import './skins/glass.css';
import './skins/fluent.css';

import { invoke } from '@tauri-apps/api/core';

import { settingsApp } from './settingsApp';
import type { Bootstrap } from './types';

// The page is the settings window; the panel is drawn natively.
await settingsApp(await invoke<Bootstrap>('bootstrap'));
