import type { ReactNode } from 'react';
import { Group, Title } from '@mantine/core';

export function PageHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <Group justify="space-between" align="center" px="xl" pt="xl" pb="lg" wrap="nowrap">
      <Title order={1}>{title}</Title>
      {children}
    </Group>
  );
}
