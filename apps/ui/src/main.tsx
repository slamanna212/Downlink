import '@mantine/core/styles.css';
import '@mantine/spotlight/styles.css';
import '@mantine/notifications/styles.css';
import './styles/global.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import { initSpatialNavigation } from './lib/spatial';

initSpatialNavigation();

// No native webview menu (Reload / Inspect). Channel rows open the app context menu instead.
document.addEventListener('contextmenu', (e) => e.preventDefault());

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
