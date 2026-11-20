import { DLButton } from '../DLButton/DLButton';
import { t } from '../../lib/i18n';
import classes from './ErrorState.module.css';

/** One sentence + one action (PLAN §8.7). Never blank, never a dead end. */
export function ErrorState({ message, onRetry, actionLabel }: { message?: string; onRetry: () => void; actionLabel?: string }) {
  return (
    <div className={classes.root} role="alert">
      <span className={classes.message}>{message ?? t('error.load')}</span>
      <DLButton variant="default" onClick={onRetry}>
        {actionLabel ?? t('error.retry')}
      </DLButton>
    </div>
  );
}
