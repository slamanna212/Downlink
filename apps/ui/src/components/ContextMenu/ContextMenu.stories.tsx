import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { Menu } from '@mantine/core';
import { IconStar } from '@tabler/icons-react';
import { Icon } from '../Icon/Icon';
import { ContextMenu } from './ContextMenu';

const meta: Meta<typeof ContextMenu> = { title: 'Primitives/ContextMenu', component: ContextMenu };
export default meta;
type Story = StoryObj<typeof ContextMenu>;

function Demo({ favourite, disabled }: { favourite: boolean; disabled?: boolean }) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>({ x: 120, y: 80 });
  return (
    <div style={{ height: 240, width: 400, border: '1px dashed gray', padding: 8 }} onContextMenu={(e) => { e.preventDefault(); setPos({ x: e.clientX, y: e.clientY }); }}>
      Right-click anywhere in this box.
      <ContextMenu opened={pos != null} x={pos?.x ?? 0} y={pos?.y ?? 0} onClose={() => setPos(null)}>
        <Menu.Item leftSection={<Icon icon={IconStar} size={16} />} disabled={disabled}>
          {favourite ? 'Remove from favourites' : 'Add to favourites'}
        </Menu.Item>
      </ContextMenu>
    </div>
  );
}

/** Open at rest; hover and keyboard focus (arrow keys / Tab) are Mantine item states. */
export const Rest: Story = { render: () => <Demo favourite={false} /> };
export const Remove: Story = { render: () => <Demo favourite /> };
export const Disabled: Story = { render: () => <Demo favourite={false} disabled /> };
