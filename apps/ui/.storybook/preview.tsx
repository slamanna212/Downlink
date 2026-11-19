import '@mantine/core/styles.css';
import '../src/styles/global.css';
import { useEffect } from 'react';
import type { Preview } from '@storybook/react-vite';
import { MantineProvider } from '@mantine/core';
import { cssVariablesResolver, dl, theme } from '../src/theme';
import { applyDensity, useUiStore } from '../src/store/ui';
import type { DensitySetting } from '../src/api/types';

const preview: Preview = {
  parameters: {
    layout: 'fullscreen',
    controls: { expanded: true },
    backgrounds: { disabled: true },
  },
  globalTypes: {
    density: {
      description: 'Density',
      toolbar: {
        title: 'Density',
        items: ['compact', 'comfortable', 'tv'],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { density: 'comfortable' },
  decorators: [
    (Story, ctx) => {
      const density = (ctx.globals.density as DensitySetting) ?? 'comfortable';
      useEffect(() => {
        useUiStore.setState({ density });
        applyDensity(density);
      }, [density]);
      return (
        <MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark" forceColorScheme="dark">
          <div className="dl-shell" data-allow-blur={String(dl.allowBlur)} style={{ padding: 24, minHeight: '100vh' }}>
            <Story />
          </div>
        </MantineProvider>
      );
    },
  ],
};
export default preview;
