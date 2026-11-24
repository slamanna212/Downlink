import { expect, test } from 'vitest';
import { orderGroups } from './groupOrder';

test('provider order groups by source and follows the latest provider positions without mutating A–Z results', () => {
  const groups = [
    { id: 1, sourceId: 1, name: 'Alpha', position: 1, count: 2 },
    { id: 3, sourceId: 2, name: 'Beta', position: 0, count: 5 },
    { id: 2, sourceId: 1, name: 'Zulu', position: 0, count: 3 },
  ];
  expect(orderGroups(groups, 'provider').map((g) => g.id)).toEqual([2, 1, 3]);
  expect(orderGroups(groups, 'alphabetical').map((g) => g.id)).toEqual([1, 3, 2]);
});
