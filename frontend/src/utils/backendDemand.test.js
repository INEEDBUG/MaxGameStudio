import { describe, expect, test } from 'vitest';
import { routeNeedsBackend } from './backendDemand';

describe('CS workspace backend demand', () => {
  test.each(['/cs2', '/cs2/'])('navigation-only %s does not start Python', path => {
    expect(routeNeedsBackend(path)).toBe(false);
  });
  test.each(['/cs2/guide', '/cs2/library', '/cs2/analysis', '/cs2/queue', '/cs2/cosmetic-studio', '/cs2/lite-cut'])('functional %s still starts Python', path => {
    expect(routeNeedsBackend(path)).toBe(true);
  });
});
