import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Collapse, PasswordInput, Stack, Switch, Tabs, TextInput } from '@mantine/core';
import { IconAlertTriangle, IconArrowRight, IconCircleCheck } from '@tabler/icons-react';
import { toAppError, invoke } from '../../api/ipc';
import { useCaps } from '../../api/queries';
import type { AddSourceInput, AppError, SourceKind, TestSourceResult } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Icon } from '../../components/Icon/Icon';
import logoUrl from '../../assets/logo.svg';
import { t } from '../../lib/i18n';
import classes from './AddSource.module.css';

export interface AddForm {
  url: string;
  apiKey: string;
  username: string;
  password: string;
  xmltvUrl: string;
  name: string;
  allowSelfSigned: boolean;
  useLogin: boolean;
}

const EMPTY: AddForm = { url: '', apiKey: '', username: '', password: '', xmltvUrl: '', name: '', allowSelfSigned: false, useLogin: false };

/** Build the IPC input for a tab. Pure, so it is unit-tested. */
export function buildInput(kind: SourceKind, f: AddForm): AddSourceInput {
  const base = { kind, name: f.name.trim() || defaultName(kind, f.url), url: f.url.trim(), allowSelfSigned: f.allowSelfSigned };
  if (kind === 'dispatcharr') return f.useLogin ? { ...base, username: f.username.trim(), password: f.password } : { ...base, apiKey: f.apiKey.trim() };
  if (kind === 'xtream') return { ...base, username: f.username.trim(), password: f.password };
  return { ...base, ...(f.xmltvUrl.trim() ? { xmltvUrl: f.xmltvUrl.trim() } : {}) };
}

export function defaultName(kind: SourceKind, url: string): string {
  try {
    return new URL(url.trim()).hostname || kind;
  } catch {
    return kind === 'dispatcharr' ? 'Dispatcharr' : kind === 'xtream' ? 'Xtream' : 'Playlist';
  }
}

/** Returns a message key-resolved string for the first problem, or null. */
export function validate(kind: SourceKind, f: AddForm): string | null {
  if (!f.name.trim()) return t('add.needName');
  if (!f.url.trim()) return t('add.needUrl');
  if (kind === 'dispatcharr' && !f.useLogin && !f.apiKey.trim()) return t('add.needKey');
  if ((kind === 'xtream' || (kind === 'dispatcharr' && f.useLogin)) && (!f.username.trim() || !f.password)) return t('add.needLogin');
  return null;
}

type Test = { state: 'idle' } | { state: 'testing' } | { state: 'ok'; result: TestSourceResult } | { state: 'error'; message: string };

export function AddSourcePage() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { hasSource } = useCaps();
  const [kind, setKind] = useState<SourceKind>('dispatcharr');
  const [form, setForm] = useState<AddForm>(EMPTY);
  const [test, setTest] = useState<Test>({ state: 'idle' });
  const [advanced, setAdvanced] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [saveError, setSaveError] = useState<AppError | null>(null);

  const set = <K extends keyof AddForm>(k: K, v: AddForm[K]) => {
    setForm((f) => ({ ...f, [k]: v }));
    setTest({ state: 'idle' }); // any edit invalidates the last test
    setSaveError(null);
  };

  const runTest = async () => {
    setAttempted(true);
    const problem = validate(kind, form);
    if (problem) return setTest({ state: 'error', message: problem });
    setTest({ state: 'testing' });
    try {
      setTest({ state: 'ok', result: await invoke('test_source', buildInput(kind, form)) });
    } catch (e) {
      setTest({ state: 'error', message: toAppError(e).message });
    }
  };

  const save = useMutation({
    mutationFn: () => invoke('add_source', buildInput(kind, form)),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ['catalog'] });
      void navigate('/channels', { state: { focusFirst: true } });
    },
    onError: (e) => setSaveError(toAppError(e)),
  });

  const tested = test.state === 'ok';
  const err = test.state === 'error' ? test.message : saveError?.message;

  return (
    <div className={classes.page}>
      <form
        className={classes.column}
        onSubmit={(e) => {
          e.preventDefault();
          if (tested) save.mutate();
          else void runTest();
        }}
      >
        <div className={classes.mark}>
          <img src={logoUrl} alt="" width={36} height={36} draggable={false} />
        </div>
        <h1 className={classes.title}>{t('add.title')}</h1>
        <p className={classes.intro}>{t('add.intro')}</p>

        <div className={classes.card}>
          <Tabs
            value={kind}
            onChange={(v) => {
              setKind((v as SourceKind) ?? 'dispatcharr');
              setTest({ state: 'idle' });
              setSaveError(null);
            }}
            variant="unstyled"
          >
            <Tabs.List className={classes.list}>
              <Tabs.Tab value="dispatcharr" className={classes.tab}>{t('add.tab.dispatcharr')}</Tabs.Tab>
              <Tabs.Tab value="xtream" className={classes.tab}>{t('add.tab.xtream')}</Tabs.Tab>
              <Tabs.Tab value="m3u" className={classes.tab}>{t('add.tab.m3u')}</Tabs.Tab>
            </Tabs.List>
          </Tabs>

          <Stack gap="lg">
            <TextInput
              label={t('add.name')}
              placeholder={t('add.namePlaceholder')}
              value={form.name}
              onChange={(e) => set('name', e.currentTarget.value)}
              error={attempted && !form.name.trim() ? t('add.needName') : undefined}
              required
              autoFocus
              autoComplete="off"
            />

            <TextInput
              className={classes.mono}
              label={kind === 'm3u' ? t('add.playlistUrl') : t('add.serverUrl')}
              placeholder={kind === 'm3u' ? 'https://example.invalid/playlist.m3u' : 'http://192.0.2.10:9191'}
              value={form.url}
              onChange={(e) => set('url', e.currentTarget.value)}
              spellCheck={false}
              autoComplete="off"
            />

            {kind === 'dispatcharr' && !form.useLogin && (
              <div className={classes.field}>
                <PasswordInput
                  className={classes.mono}
                  label={t('add.apiKey')}
                  description={t('add.apiKeyHelp')}
                  inputWrapperOrder={['label', 'input', 'description', 'error']}
                  value={form.apiKey}
                  onChange={(e) => set('apiKey', e.currentTarget.value)}
                  autoComplete="off"
                />
                <button type="button" className={`${classes.link} ${classes.fieldLink}`} onClick={() => set('useLogin', true)}>
                  {t('add.useLogin')}
                </button>
              </div>
            )}

            {(kind === 'xtream' || (kind === 'dispatcharr' && form.useLogin)) && (
              <>
                <TextInput label={t('add.username')} value={form.username} onChange={(e) => set('username', e.currentTarget.value)} autoComplete="off" spellCheck={false} />
                <PasswordInput
                  label={t('add.password')}
                  description={t('add.passwordHelp')}
                  inputWrapperOrder={['label', 'input', 'description', 'error']}
                  value={form.password}
                  onChange={(e) => set('password', e.currentTarget.value)}
                  autoComplete="off"
                />
                {kind === 'dispatcharr' && (
                  <button type="button" className={`${classes.link} ${classes.start}`} onClick={() => set('useLogin', false)}>
                    {t('add.useKey')}
                  </button>
                )}
              </>
            )}

            {kind === 'm3u' && (
              <TextInput
                className={classes.mono}
                label={t('add.xmltvUrl')}
                placeholder="https://example.invalid/guide.xml"
                value={form.xmltvUrl}
                onChange={(e) => set('xmltvUrl', e.currentTarget.value)}
                spellCheck={false}
                autoComplete="off"
              />
            )}

            <button type="button" className={`${classes.link} ${classes.start}`} aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)}>
              {t('add.advanced')}
            </button>
            <Collapse expanded={advanced}>
              <Stack gap="lg">
                <Switch
                  label={t('add.selfSigned')}
                  description={t('add.selfSignedHelp')}
                  checked={form.allowSelfSigned}
                  onChange={(e) => set('allowSelfSigned', e.currentTarget.checked)}
                />
              </Stack>
            </Collapse>

            {tested && (
              <div className={`${classes.result} ${classes.resultOk}`} role="status">
                <span className={classes.resultIcon}><Icon icon={IconCircleCheck} size={20} /></span>
                <span className={classes.resultText}>
                  {test.result.user
                    ? t('add.connectedAs', { server: serverLabel(kind, test.result), user: test.result.user })
                    : t('add.connected', { server: serverLabel(kind, test.result) })}
                </span>
                <span className={classes.resultCounts}>
                  {t('add.counts', { channels: test.result.channels.toLocaleString('en'), programmes: test.result.programmes.toLocaleString('en') })}
                </span>
              </div>
            )}
            {err && (
              <div className={`${classes.result} ${classes.resultErr}`} role="alert">
                <span className={classes.resultIcon}><Icon icon={IconAlertTriangle} size={20} /></span>
                <span className={classes.resultText}>{err}</span>
              </div>
            )}
          </Stack>

          <div className={classes.actions}>
            {hasSource && (
              <DLButton variant="subtle" onClick={() => void navigate(-1)}>
                {t('add.cancel')}
              </DLButton>
            )}
            {tested ? (
              <>
                <DLButton variant="default" onClick={() => void runTest()}>{t('add.testAgain')}</DLButton>
                <DLButton variant="filled" type="submit" loading={save.isPending}>
                  {t('add.save')}
                  {!save.isPending && <span className={classes.arrow}><Icon icon={IconArrowRight} size={16} /></span>}
                </DLButton>
              </>
            ) : (
              <DLButton variant="filled" type="submit" loading={test.state === 'testing'}>
                {t('add.test')}
              </DLButton>
            )}
          </div>
        </div>
        <p className={classes.footer}>{t('add.footer')}</p>
      </form>
    </div>
  );
}

function serverLabel(kind: SourceKind, r: TestSourceResult): string {
  const name = kind === 'dispatcharr' ? 'Dispatcharr' : kind === 'xtream' ? 'Xtream' : 'playlist';
  return r.serverVersion ? `${name} ${r.serverVersion}` : name;
}
