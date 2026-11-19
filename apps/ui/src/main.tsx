import '@mantine/core/styles.css';
import './styles/global.css';
import { createRoot } from 'react-dom/client';
import { MantineProvider } from '@mantine/core';
import { theme, dl, cssVariablesResolver } from './theme';

createRoot(document.getElementById('root')!).render(
  <MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark">
    <div className="dl-shell" data-allow-blur={String(dl.allowBlur)} />
  </MantineProvider>,
);
