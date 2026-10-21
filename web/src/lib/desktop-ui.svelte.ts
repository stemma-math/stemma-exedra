/** The shell's desktop hooks; the browser build has no native imports. */
export const desktopUI = $state({
  signOut: async () => {},
  serviceConnected: null as boolean | null,
  serviceRunning: false,
  machineNotKnown: false,
});
