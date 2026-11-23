import { beforeEach, describe, expect, it } from 'vitest';
import { blockingKind, noteBlockingError, useBlockingStore } from './blocking';

describe('blocking prompts', () => {
  beforeEach(() => useBlockingStore.getState().clear());
  it('recognises database and certificate prompts', () => {
    expect(blockingKind('This library was created by a newer version of Downlink.')).toBe('newer-db');
    expect(blockingKind('The server’s certificate changed. Trust it?')).toBe('cert');
    expect(blockingKind("The server's certificate changed.")).toBe('cert');
    expect(blockingKind('This source has no trusted certificate.')).toBe('missing-cert');
    expect(blockingKind('Could not reach the server.')).toBeNull();
  });
  it('raises the screen from a failed command', () => {
    noteBlockingError('Could not reach the server.');
    expect(useBlockingStore.getState().kind).toBeNull();
    noteBlockingError("The server's certificate changed.", 42);
    expect(useBlockingStore.getState().kind).toBe('cert');
    expect(useBlockingStore.getState().sourceId).toBe(42);
  });
});
