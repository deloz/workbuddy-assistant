<template>
  <section class="view-panel settings-view" aria-labelledby="settings-title">
    <header class="view-heading">
      <div>
        <p class="section-kicker">应用偏好</p>
        <h1 id="settings-title">设置</h1>
      </div>
    </header>

    <section class="settings-section" aria-labelledby="about-title">
      <h2 id="about-title">关于</h2>
      <dl class="about-list">
        <div class="about-row about-row--version">
          <dt>版本</dt>
          <dd>
            <span class="settings-version">{{ store.version ? `v${store.version}` : "—" }}</span>
            <button
              class="button button--secondary"
              type="button"
              :disabled="store.loading || store.checking"
              @click="store.checkForUpdates"
            >
              <span v-if="store.checking" class="button-spinner" aria-hidden="true"></span>
              {{ store.checking ? "检查中…" : "检查更新" }}
            </button>
            <p
              v-if="updateMessage"
              class="settings-message"
              :class="`settings-message--${updateMessageTone}`"
              role="status"
              aria-live="polite"
            >
              {{ updateMessage }}
            </p>
            <button
              v-if="store.updateResult?.status === 'updateAvailable'"
              class="button button--primary"
              type="button"
              @click="openLatestRelease"
            >
              前往下载
            </button>
          </dd>
        </div>
        <div class="about-row">
          <dt>作者</dt>
          <dd>Deloz</dd>
        </div>
        <div class="about-row about-row--links">
          <dt>相关链接</dt>
          <dd>
            <button
              v-for="link in links"
              :key="link.url"
              class="button button--secondary settings-link"
              type="button"
              @click="store.openExternalLink(link.url)"
            >
              {{ link.label }}
            </button>
          </dd>
        </div>
        <div v-if="store.linkError" class="about-row about-row--message">
          <dt>链接状态</dt>
          <dd>
            <p class="settings-message settings-message--error" role="status">
              {{ store.linkError }}
            </p>
          </dd>
        </div>
      </dl>
    </section>
  </section>
</template>

<script setup>
import { computed, onMounted } from "vue";

import { EXTERNAL_LINKS, useSettingsStore } from "../../stores/settings";
import { formatUpdateMessage, getUpdateMessageTone } from "../../utils/appState";

const store = useSettingsStore();
const links = [
  { label: "项目仓库", url: EXTERNAL_LINKS.repository },
];

const updateMessage = computed(() =>
  formatUpdateMessage({
    checking: store.checking,
    error: store.updateError,
    result: store.updateResult,
  }),
);
const updateMessageTone = computed(() =>
  getUpdateMessageTone({
    error: store.updateError,
    result: store.updateResult,
  }),
);

function openLatestRelease() {
  const releaseUrl = store.updateResult?.releaseUrl;
  if (releaseUrl) {
    store.openExternalLink(releaseUrl);
  }
}

onMounted(() => {
  store.initialize();
});
</script>
