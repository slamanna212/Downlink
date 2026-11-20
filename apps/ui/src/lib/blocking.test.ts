import { beforeEach, describe, expect, it } from 'vitest';
import { blockingKind, noteBlockingError, useBlockingStore } from './blocking';

describe('blocking prompts', () => {
  beforeEach(() => useBlockingStore.getState().clear());
  it('recognises the two messages (curly or straight apostrophe) and nothing else', () => {
    expect(blockingKind('This library was created by a newer version of Downlink.')).toBe('newer-db');
    expect(blockingKind('The server’s certificate changed. Trust it?')).toBe('cert');
    expect(blockingKind("The server's certificate changed.")).toBe('cert');
    expect(blockingKind('Could not reach the server.')).toBeNull();
  });
  it('raises the screen from a failed command', () => {
    noteBlockingError('Could not reach the server.');
    expect(useBlockingStore.getState().kind).toBeNull();
    noteBlockingError("The server's certificate changed.");
    expect(useBlockingStore.getState().kind).toBe('cert');
  });
});
