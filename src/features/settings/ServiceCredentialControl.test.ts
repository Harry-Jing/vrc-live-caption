// @vitest-environment happy-dom

import ui from "@nuxt/ui/vue-plugin";
import { mount } from "@vue/test-utils";
import { describe, expect, test } from "vitest";
import ServiceCredentialControl from "./ServiceCredentialControl.vue";
import type { ServiceCredentialControlCopy } from "./serviceCredentialControl";

const copy: ServiceCredentialControlCopy = {
  title: "Custom Translation credentials",
  disclosure: "Stored separately in the system credential store.",
  inputLabel: "API key",
  inputPlaceholder: "Custom API key",
  save: "Save key",
  replace: "Replace key",
  remove: "Remove key",
  actionFailed: "Custom credential action failed",
  removeDialogTitle: "Remove Custom Translation API key?",
  removeDialogDescription: "The saved key will be removed.",
  removeDialogCurrentGenerationDescription:
    "The current run keeps its captured key until Stop.",
  removeDialogCancel: "Cancel",
  removeDialogConfirm: "Remove API key",
};

function mountControl(
  overrides: Partial<
    InstanceType<typeof ServiceCredentialControl>["$props"]
  > = {},
) {
  return mount(ServiceCredentialControl, {
    props: {
      actionFailure: "",
      busy: false,
      capturedByActiveGeneration: false,
      copy,
      status: { state: "unconfigured", id: "customTranslation" },
      ...overrides,
    },
    global: { plugins: [ui] },
    attachTo: document.body,
  });
}

describe("service credential control", () => {
  test.each([
    [null, "Checking", false, "Save key"],
    [
      { state: "unconfigured", id: "customTranslation" } as const,
      "Not saved",
      false,
      "Save key",
    ],
    [
      {
        state: "configured",
        id: "customTranslation",
        storage: "systemCredentialStore",
        displaySuffix: "abcd",
      } as const,
      "System ...abcd",
      true,
      "Replace key",
    ],
    [
      {
        state: "configured",
        id: "openai",
        storage: "environment",
        displaySuffix: "wxyz",
      } as const,
      "Environment ...wxyz",
      false,
      "Save key",
    ],
  ])(
    "presents status %# without exposing a removable environment key",
    (status, label, canRemove, saveLabel) => {
      const wrapper = mountControl({ status });

      expect(wrapper.text()).toContain(label);
      expect(wrapper.text()).toContain(saveLabel);
      expect(wrapper.text().includes("Remove key")).toBe(canRemove);

      wrapper.unmount();
    },
  );

  test("presents unavailable status and operation failures independently", () => {
    const wrapper = mountControl({
      actionFailure: "Could not replace the key.",
      status: {
        state: "unavailable",
        id: "customTranslation",
        failure: {
          code: "config.secret_failed",
          message: "Credential store unavailable.",
        },
      },
    });

    expect(wrapper.text()).toContain("Unavailable");
    expect(wrapper.text()).toContain("Credential store unavailable.");
    expect(wrapper.text()).toContain("Custom credential action failed");
    expect(wrapper.text()).toContain("Could not replace the key.");
    expect(wrapper.findAll('[role="alert"]')).toHaveLength(2);

    wrapper.unmount();
  });

  test("Enter saves only this credential and clears plaintext immediately", async () => {
    const wrapper = mountControl();
    const input = wrapper.get('input[type="password"]');

    await input.setValue("custom-secret-abcd");
    await input.trigger("keydown", { key: "Enter" });

    expect(wrapper.emitted("save")).toEqual([["custom-secret-abcd"]]);
    expect((input.element as HTMLInputElement).value).toBe("");
    expect(wrapper.html()).not.toContain("custom-secret-abcd");

    wrapper.unmount();
  });

  test("confirms deletion and explains the active-generation boundary", async () => {
    const wrapper = mountControl({
      capturedByActiveGeneration: true,
      status: {
        state: "configured",
        id: "customTranslation",
        storage: "systemCredentialStore",
        displaySuffix: "abcd",
      },
    });
    const remove = wrapper
      .findAll("button")
      .find((button) => button.text().includes("Remove key"));

    if (!remove) {
      throw new Error("Remove key action was not rendered.");
    }
    await remove.trigger("click");
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(document.body.textContent).toContain(
      "The current run keeps its captured key until Stop.",
    );

    const confirm = Array.from(document.body.querySelectorAll("button")).find(
      (button) => button.textContent.includes("Remove API key"),
    );
    if (!confirm) {
      throw new Error("Remove key confirmation was not rendered.");
    }
    confirm.click();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(wrapper.emitted("delete")).toEqual([[]]);

    wrapper.unmount();
  });
});
