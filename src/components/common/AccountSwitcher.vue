<template>
  <div class="account-switcher">
    <label class="account-switcher__field" for="account-switcher-select">
      <span>查看账号</span>
      <select
        id="account-switcher-select"
        :value="selectedAccountId || ''"
        :disabled="disabled || accounts.length === 0"
        aria-describedby="account-switcher-details"
        @change="emit('select', $event.target.value)"
      >
        <option v-for="account in accounts" :key="account.accountId" :value="account.accountId">
          {{ account.nickname }} · {{ accountTypeLabel(account.accountType) }}
        </option>
      </select>
    </label>

    <div v-if="selectedAccount" id="account-switcher-details" class="account-switcher__details">
      <span>{{ accountTypeLabel(selectedAccount.accountType) }}</span>
      <span aria-hidden="true">·</span>
      <span>账号 ID {{ visibleAccountId }}</span>
      <button
        class="account-switcher__reveal"
        type="button"
        :aria-pressed="Boolean(revealedAccountIds[selectedAccount.accountId])"
        @click="emit('reveal', selectedAccount.accountId)"
      >
        {{ revealedAccountIds[selectedAccount.accountId] ? "隐藏" : "显示完整 ID" }}
      </button>
    </div>
  </div>
</template>

<script setup>
import { computed } from "vue";

const props = defineProps({
  accounts: { type: Array, default: () => [] },
  selectedAccountId: { type: String, default: null },
  revealedAccountIds: { type: Object, default: () => ({}) },
  disabled: { type: Boolean, default: false },
});

const emit = defineEmits(["select", "reveal"]);

const selectedAccount = computed(
  () =>
    props.accounts.find((account) => account.accountId === props.selectedAccountId) ||
    props.accounts[0] ||
    null,
);

const visibleAccountId = computed(() => {
  const account = selectedAccount.value;
  if (!account) return "—";
  return props.revealedAccountIds[account.accountId] || account.maskedAccountId;
});

function accountTypeLabel(accountType) {
  return String(accountType).toLowerCase() === "personal"
    ? "个人账号"
    : "企业账号";
}
</script>
