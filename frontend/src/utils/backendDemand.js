// Only routes with local service work should start Python. Unknown/legacy
// routes redirect before making a demand; the shell and League stay usable.
export function routeNeedsBackend(pathname) {
  return (
    pathname.startsWith('/cs2/') ||
    pathname.startsWith('/valorant/') || pathname.startsWith('/peripherals/') ||
    pathname === '/settings' || pathname.startsWith('/obs-ai')
  );
}
