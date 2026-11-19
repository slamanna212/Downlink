// Storybook helper: lays out the six required states (DESIGN §11 rule 5) for a component.
import type { ReactNode } from 'react';
import { Group, Stack, Text } from '@mantine/core';

export const STATES = ['rest', 'hover', 'pressed', 'focus', 'disabled', 'loading'] as const;
export type DemoState = (typeof STATES)[number];

/** Props to spread on the demo element so the pseudo-states addon / data-focused force the state. */
export function demoStateProps(state: DemoState): Record<string, unknown> {
  return {
    'data-demo-state': state,
    ...(state === 'focus' ? { 'data-focused': 'true' } : {}),
    ...(state === 'disabled' ? { disabled: true } : {}),
    ...(state === 'loading' ? { loading: true } : {}),
  };
}

export function StateRow({ title, children }: { title: string; children: (state: DemoState) => ReactNode }) {
  return (
    <Stack gap="xs">
      <Text size="xs" fw={600} c="dimmed" tt="uppercase" style={{ letterSpacing: '0.06em' }}>
        {title}
      </Text>
      <Group gap="xl" align="center">
        {STATES.map((s) => (
          <Stack key={s} gap={6} align="flex-start">
            {children(s)}
            <Text size="xs" c="dimmed">
              {s}
            </Text>
          </Stack>
        ))}
      </Group>
    </Stack>
  );
}

export const pseudoStates = {
  pseudo: {
    hover: ['[data-demo-state="hover"]'],
    active: ['[data-demo-state="pressed"]'],
  },
};
