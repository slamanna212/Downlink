import type { ReactNode } from 'react';
import classes from './Settings.module.css';

/** A settings page section: title, one line of help, a hairline, then its rows. */
export function Section({ title, help, children }: { title: string; help: string; children: ReactNode }) {
  return (
    <section>
      <h2 className={classes.sectionTitle}>{title}</h2>
      <p className={classes.sectionHelp}>{help}</p>
      <div className={classes.divider} />
      {children}
    </section>
  );
}
