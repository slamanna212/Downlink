import { describe, expect, it } from 'vitest';
import { buildInput, defaultName, validate, type AddForm } from './AddSourcePage';

const base: AddForm = { url: ' http://demo.invalid:9191 ', apiKey: ' key ', username: ' u ', password: 'p', xmltvUrl: '', name: '', allowSelfSigned: true, useLogin: false };

describe('add source form', () => {
  it('builds a Dispatcharr API-key input, trimmed, with a default name', () => {
    expect(buildInput('dispatcharr', base)).toEqual({ kind: 'dispatcharr', name: 'demo.invalid', url: 'http://demo.invalid:9191', apiKey: 'key', allowSelfSigned: true });
  });
  it('switches to username/password for Dispatcharr and Xtream', () => {
    expect(buildInput('dispatcharr', { ...base, useLogin: true })).toMatchObject({ username: 'u', password: 'p' });
    expect(buildInput('dispatcharr', { ...base, useLogin: true })).not.toHaveProperty('apiKey');
    expect(buildInput('xtream', base)).toMatchObject({ kind: 'xtream', username: 'u', password: 'p' });
  });
  it('includes the XMLTV URL for M3U only when set', () => {
    expect(buildInput('m3u', base)).not.toHaveProperty('xmltvUrl');
    expect(buildInput('m3u', { ...base, xmltvUrl: 'http://x.invalid/g.xml' })).toMatchObject({ xmltvUrl: 'http://x.invalid/g.xml' });
  });
  it('validates required fields per tab', () => {
    expect(validate('dispatcharr', { ...base, url: '' })).toBeTruthy();
    expect(validate('dispatcharr', { ...base, apiKey: '' })).toBeTruthy();
    expect(validate('dispatcharr', base)).toBeNull();
    expect(validate('xtream', { ...base, password: '' })).toBeTruthy();
    expect(validate('m3u', { ...base, apiKey: '', username: '' })).toBeNull();
  });
  it('falls back to a generic name for unparsable URLs', () => {
    expect(defaultName('xtream', 'not a url')).toBe('Xtream');
  });
});
