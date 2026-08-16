import { ref } from "vue";
import { defineStore } from "pinia";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { normalizeError } from "../utils/appState";

export const EXTERNAL_LINKS = Object.freeze({
  repository: "https://github.com/deloz/workbuddy-assistant",
  latestRelease: "https://github.com/deloz/workbuddy-assistant/releases/latest",
});

const allowedLinks = new Set(Object.values(EXTERNAL_LINKS));

export const useSettingsStore = defineStore("settings", () => {
  const version = ref("");
  const initialized = ref(false);
  const loading = ref(false);
  const checking = ref(false);
  const updateError = ref(null);
  const updateResult = ref(null);
  const linkError = ref(null);

  async function initialize() {
    if (initialized.value) {
      return true;
    }
    if (loading.value) {
      return false;
    }
    loading.value = true;
    try {
      version.value = await getVersion();
      initialized.value = true;
      return true;
    } catch {
      return false;
    } finally {
      loading.value = false;
    }
  }

  async function checkForUpdates() {
    if (checking.value) {
      return false;
    }
    checking.value = true;
    updateError.value = null;
    updateResult.value = null;
    try {
      updateResult.value = await invoke("check_for_updates");
      return true;
    } catch (reason) {
      updateError.value = normalizeError(reason);
      return false;
    } finally {
      checking.value = false;
    }
  }

  async function openExternalLink(url) {
    linkError.value = null;
    if (!allowedLinks.has(url)) {
      linkError.value = "该链接不在允许打开的范围内";
      return false;
    }
    try {
      await openUrl(url);
      return true;
    } catch (reason) {
      linkError.value = normalizeError(reason);
      return false;
    }
  }

  return {
    initialized,
    loading,
    checking,
    updateError,
    updateResult,
    linkError,
    version,
    initialize,
    checkForUpdates,
    openExternalLink,
  };
});
