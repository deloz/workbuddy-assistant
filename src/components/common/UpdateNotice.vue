<template>
  <Teleport to="body">
    <aside v-if="store.updateNoticeVisible" class="update-notice" role="status" aria-live="polite">
      <button class="update-notice__action" type="button" @click="store.openUpdateGuide">
        <span class="update-clock" aria-hidden="true"></span>
        <strong>新版本可用: v{{ store.updateResult.latestVersion }}</strong>
      </button>
      <button
        class="update-close"
        type="button"
        aria-label="忽略本次更新提示"
        title="关闭"
        @click="store.dismissUpdate"
      >
        ×
      </button>
    </aside>

    <section
      v-if="store.updateGuideOpen"
      class="update-guide"
      role="dialog"
      aria-labelledby="update-guide-title"
    >
      <header class="update-guide__header">
        <div class="update-guide__title">
          <span class="update-clock" aria-hidden="true"></span>
          <h2 id="update-guide-title">新版本可用: v{{ store.updateResult.latestVersion }}</h2>
        </div>
        <button
          ref="guideCloseButton"
          class="update-close"
          type="button"
          aria-label="关闭更新指导"
          title="关闭"
          @click="store.dismissUpdate"
        >
          ×
        </button>
      </header>

      <dl class="update-guide__versions">
        <div>
          <dt>当前版本:</dt>
          <dd>v{{ store.updateResult.currentVersion }}</dd>
        </div>
        <div>
          <dt>最新版本:</dt>
          <dd class="update-guide__latest">v{{ store.updateResult.latestVersion }}</dd>
        </div>
      </dl>

      <div class="update-guide__body">
        <h3>安装方式:</h3>

        <button class="update-release" type="button" @click="openLatestRelease">
          <span>GitHub Release</span>
          <span>打开下载页</span>
        </button>
        <p class="update-guide__hint">
          下载名称以 <code>-setup.exe</code> 结尾的安装器；安装或执行 Scoop 更新前，请先从系统托盘退出 WorkBuddy助手。
        </p>

        <div class="update-command">
          <span class="update-command__label">Scoop 命令</span>
          <div class="update-command__line">
            <code>{{ SCOOP_UPDATE_COMMAND }}</code>
            <button type="button" @click="copyScoopCommand">
              {{ copied ? "已复制" : "复制" }}
            </button>
          </div>
        </div>

        <p v-if="copyError" class="update-guide__error" role="status">{{ copyError }}</p>
        <p v-if="store.linkError" class="update-guide__error" role="status">{{ store.linkError }}</p>
      </div>
    </section>
  </Teleport>
</template>

<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

import { useSettingsStore } from "../../stores/settings";

const SCOOP_UPDATE_COMMAND = "scoop update workbuddy-assistant";

const store = useSettingsStore();
const guideCloseButton = ref(null);
const copied = ref(false);
const copyError = ref("");
let copyResetTimer = null;
let previousFocusedElement = null;

async function openLatestRelease() {
  await store.openExternalLink(store.updateResult.releaseUrl);
}

async function copyScoopCommand() {
  copyError.value = "";
  try {
    if (!navigator.clipboard?.writeText) {
      throw new Error("Clipboard API unavailable");
    }
    await navigator.clipboard.writeText(SCOOP_UPDATE_COMMAND);
    copied.value = true;
    window.clearTimeout(copyResetTimer);
    copyResetTimer = window.setTimeout(() => {
      copied.value = false;
    }, 1800);
  } catch {
    copied.value = false;
    copyError.value = "复制失败，请手动选择命令";
  }
}

function handleKeydown(event) {
  if (event.key === "Escape" && store.updateGuideOpen) {
    store.dismissUpdate();
  }
}

watch(
  () => store.updateGuideOpen,
  async (open) => {
    if (!open) {
      if (previousFocusedElement?.isConnected && typeof previousFocusedElement.focus === "function") {
        previousFocusedElement.focus();
      }
      previousFocusedElement = null;
      return;
    }

    previousFocusedElement = document.activeElement;
    copied.value = false;
    copyError.value = "";
    await nextTick();
    guideCloseButton.value?.focus();
  },
);

onMounted(() => {
  document.addEventListener("keydown", handleKeydown);
});

onBeforeUnmount(() => {
  document.removeEventListener("keydown", handleKeydown);
  window.clearTimeout(copyResetTimer);
});
</script>
