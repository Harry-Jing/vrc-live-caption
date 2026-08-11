import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { requestConfirmation } from "./confirmation";

const dialogMocks = vi.hoisted(() => ({
  confirm: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: dialogMocks.isTauri,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: dialogMocks.confirm,
}));

beforeEach(() => {
  dialogMocks.confirm.mockReset();
  dialogMocks.isTauri.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test("uses the native Tauri confirmation dialog", async () => {
  dialogMocks.isTauri.mockReturnValue(true);
  dialogMocks.confirm.mockResolvedValue(true);

  await expect(requestConfirmation("Discard changes?")).resolves.toBe(true);
  expect(dialogMocks.confirm).toHaveBeenCalledWith("Discard changes?", {
    kind: "warning",
  });
});

test("preserves cancellation from the native Tauri dialog", async () => {
  dialogMocks.isTauri.mockReturnValue(true);
  dialogMocks.confirm.mockResolvedValue(false);

  await expect(requestConfirmation("Discard changes?")).resolves.toBe(false);
});

test("uses window.confirm outside Tauri", async () => {
  const browserConfirm = vi.fn(() => true);
  vi.stubGlobal("window", { confirm: browserConfirm });
  dialogMocks.isTauri.mockReturnValue(false);

  await expect(requestConfirmation("Discard changes?")).resolves.toBe(true);
  expect(browserConfirm).toHaveBeenCalledWith("Discard changes?");
  expect(dialogMocks.confirm).not.toHaveBeenCalled();
});

test("cancels safely when the confirmation dialog fails", async () => {
  const failure = new Error("dialog unavailable");
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  dialogMocks.isTauri.mockReturnValue(true);
  dialogMocks.confirm.mockRejectedValue(failure);

  await expect(requestConfirmation("Discard changes?")).resolves.toBe(false);
  expect(consoleError).toHaveBeenCalledWith(
    "Failed to show confirmation dialog.",
    failure,
  );
});
