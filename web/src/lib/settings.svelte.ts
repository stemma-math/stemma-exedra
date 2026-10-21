// The person's own settings: one dialog, in the browser and the app.

/** The pages every face has; the app adds this machine's after them. */
export const PAGES = ["Profile", "Appearance", "Devices"] as const;
export const MACHINE_PAGES = ["This machine"] as const;
export type SettingsPage = (typeof PAGES)[number] | (typeof MACHINE_PAGES)[number];

export const settingsUI = $state<{ open: boolean; page: SettingsPage }>({
  open: false,
  page: "Appearance",
});

/** Opens the settings at `page`; "Settings" alone opens the first page the
 * account menu has no item of its own for. */
export function openSettings(page: SettingsPage = "Devices") {
  settingsUI.page = page;
  settingsUI.open = true;
}
