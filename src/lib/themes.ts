// Shapes shared by the views, mirroring the serde field names of
// src-tauri/src/commands (themes.rs and environment.rs).

export interface ThemeInfo {
  name: string;
  description: string;
  module: string;
  path: string;
  hasPreview: boolean;
  isGraphical: boolean;
}

export interface EnvironmentInfo {
  desktopEnvironment: string;
  displayProtocol: string;
  initramfs: string;
  initramfsAvailable: string[];
}
