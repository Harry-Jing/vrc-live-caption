import { isTauri } from "@tauri-apps/api/core";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";

export async function requestConfirmation(message: string): Promise<boolean> {
  try {
    if (isTauri()) {
      return await confirmDialog(message, { kind: "warning" });
    }

    return window.confirm(message);
  } catch (error) {
    console.error("Failed to show confirmation dialog.", error);
    return false;
  }
}
