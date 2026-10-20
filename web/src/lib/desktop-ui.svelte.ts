/** The shell's desktop hooks; the browser build has no native imports. */
export const desktopUI = $state({
  openSettings: () => {},
  signOut: async () => {},
  serviceConnected: null as boolean | null,
  serviceRunning: false,
});
