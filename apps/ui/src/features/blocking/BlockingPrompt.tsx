import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { invoke } from '../../api/ipc';
import { DLButton } from '../../components/DLButton/DLButton';
import { useBlockingStore } from '../../lib/blocking';
import { notifyError } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import classes from './BlockingPrompt.module.css';

/**
 * Full-screen decision for the two states the app cannot continue past (PLAN §7.6 newer database,
 * §2.6 changed certificate). The safe choice is the filled button; the destructive one is `danger`
 * (outlined), never a filled primary (DESIGN).
 */
export function BlockingPrompt() {
  const kind = useBlockingStore((s) => s.kind);
  const sourceId = useBlockingStore((s) => s.sourceId);
  const clear = useBlockingStore((s) => s.clear);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  if (!kind) return null;

  const run = async (fn: () => Promise<unknown>, after: () => void) => {
    setBusy(true);
    try {
      await fn();
      after();
    } catch (e) {
      notifyError(e);
    } finally {
      setBusy(false);
    }
  };

  const newerDb = kind === 'newer-db';
  const missingCert = kind === 'missing-cert';
  const title = newerDb ? 'blocking.newerDb.title' : missingCert ? 'blocking.cert.missingTitle' : 'blocking.cert.title';
  const body = newerDb ? 'blocking.newerDb.body' : missingCert ? 'blocking.cert.missingBody' : 'blocking.cert.body';
  return (
    <div className={classes.scrim} role="alertdialog" aria-modal="true" aria-labelledby="blocking-title" aria-describedby="blocking-body">
      <div className={classes.card}>
        <h1 id="blocking-title" className={classes.title}>{t(title)}</h1>
        <p id="blocking-body" className={classes.body}>{t(body)}</p>
        <div className={classes.actions}>
          {newerDb ? (
            <>
              <DLButton variant="filled" size="lg" disabled={busy} onClick={() => void run(() => invoke('quit_app', {}), () => undefined)}>
                {t('blocking.newerDb.quit')}
              </DLButton>
              <DLButton
                variant="danger"
                size="lg"
                loading={busy}
                onClick={() => void run(() => invoke('archive_newer_database', {}), () => window.location.reload())}
              >
                {t('blocking.newerDb.fresh')}
              </DLButton>
            </>
          ) : (
            <>
              <DLButton variant="filled" size="lg" disabled={busy} onClick={clear}>
                {t('blocking.cert.cancel')}
              </DLButton>
              {sourceId != null && (
                <DLButton
                  variant="danger"
                  size="lg"
                  loading={busy}
                  onClick={() =>
                    void run(
                      () => invoke('trust_new_certificate', { sourceId }),
                      () => {
                        clear();
                        void qc.invalidateQueries();
                      },
                    )
                  }
                >
                  {t('blocking.cert.trust')}
                </DLButton>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
