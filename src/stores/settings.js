import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { isActionableUpdate, normalizeError } from "../utils/appState";

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
  const updateGuideOpen = ref(false);
  const updateNoticeDismissed = ref(false);
  const growthAutomationEnabled = ref(true);
  const settingsLoaded = ref(false);
  const savingGrowthAutomation = ref(false);
  const settingsError = ref(null);

  const actionableUpdate = computed(() => isActionableUpdate(updateResult.value));
  const updateNoticeVisible = computed(
    () => actionableUpdate.value && !updateNoticeDismissed.value && !updateGuideOpen.value,
  );

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

  async function loadAppSettings() {
    try {
      const settings = await invoke("get_app_settings");
      growthAutomationEnabled.value = settings?.growthAutomationEnabled !== false;
      settingsLoaded.value = true;
      return true;
    } catch {
      return false;
    }
  }

  async function setGrowthAutomation(enabled) {
    if (savingGrowthAutomation.value) {
      return false;
    }
    savingGrowthAutomation.value = true;
    settingsError.value = null;
    try {
      const settings = await invoke("set_growth_automation_enabled", { enabled });
      growthAutomationEnabled.value = settings?.growthAutomationEnabled !== false;
      return true;
    } catch (reason) {
      settingsError.value = normalizeError(reason);
      return false;
    } finally {
      savingGrowthAutomation.value = false;
    }
  }

  async function toggleGrowthAutomation() {
    return setGrowthAutomation(!growthAutomationEnabled.value);
  }

  async function checkForUpdates() {
    if (checking.value) {
      return false;
    }
    checking.value = true;
    updateError.value = null;
    updateResult.value = null;
    updateGuideOpen.value = false;
    try {
      updateResult.value = await invoke("check_for_updates");
      updateNoticeDismissed.value = !isActionableUpdate(updateResult.value);
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

  function openUpdateGuide() {
    if (!actionableUpdate.value) {
      return false;
    }
    linkError.value = null;
    updateNoticeDismissed.value = false;
    updateGuideOpen.value = true;
    return true;
  }

  function dismissUpdate() {
    updateGuideOpen.value = false;
    updateNoticeDismissed.value = true;
  }

  return {
    initialized,
    loading,
    checking,
    updateError,
    updateResult,
    linkError,
    actionableUpdate,
    updateGuideOpen,
    updateNoticeVisible,
    version,
    growthAutomationEnabled,
    settingsLoaded,
    savingGrowthAutomation,
    settingsError,
    initialize,
    loadAppSettings,
    setGrowthAutomation,
    toggleGrowthAutomation,
    checkForUpdates,
    openExternalLink,
    openUpdateGuide,
    dismissUpdate,
  };
});
