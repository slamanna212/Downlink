import type { ReactNode } from 'react';
import classes from './SettingRow.module.css';

export interface SettingRowProps {
  label: string;
  help?: string;
  /** The control, 260 px wide on the right (DESIGN §7). `wide` lets it size itself. */
  children: ReactNode;
  wide?: boolean;
  /** Label id for aria-labelledby on the control. */
  labelId?: string;
}

export function SettingRow({ label, help, children, wide, labelId }: SettingRowProps) {
  return (
    <div className={classes.row} role="group" aria-labelledby={labelId}>
      <div className={classes.text}>
        <div className={classes.label} id={labelId}>{label}</div>
        {help && <div className={classes.help}>{help}</div>}
      </div>
      <div className={classes.control} data-wide={wide || undefined}>{children}</div>
    </div>
  );
}
