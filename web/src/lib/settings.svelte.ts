// The person's own settings: one dialog, in the browser and the app.

/** The pages every face has; the app adds this machine's after them. */
export const PAGES = ["Profile", "Notifications", "Appearance", "Devices"] as const;
export const MACHINE_PAGES = ["This machine"] as const;
export type SettingsPage = (typeof PAGES)[number] | (typeof MACHINE_PAGES)[number];

export const settingsUI = $state<{ open: boolean; page: SettingsPage }>({
  open: false,
  page: "Profile",
});

/** Opens the settings at `page`; "Settings" alone opens the profile, or in
 * the app this machine's. */
export function openSettings(page: SettingsPage = __STEMMA_DESKTOP__ ? "This machine" : "Profile") {
  settingsUI.page = page;
  settingsUI.open = true;
}
