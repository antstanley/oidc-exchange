<script lang="ts">
    let { children } = $props();

    const nav = [
        { href: '/', label: 'Dashboard', icon: '&#x1F4CA;' },
        { href: '/users', label: 'Users', icon: '&#x1F465;' },
    ];
</script>

<div class="min-h-screen bg-gray-950 flex">
  <!-- Sidebar -->
  <aside class="w-56 bg-gray-900 border-r border-gray-800 flex flex-col">
    <div class="p-4 border-b border-gray-800">
      <h1 class="text-lg font-bold text-white">oidc-exchange</h1>
      <p class="text-xs text-gray-500">Admin Console</p>
    </div>
    <nav class="flex-1 p-3 space-y-1">
      {#each nav as item}
        <a href={item.href} class="flex items-center gap-2 px-3 py-2 rounded-lg text-gray-300 hover:bg-gray-800 hover:text-white text-sm transition-colors">
          <span>{@html item.icon}</span>
          {item.label}
        </a>
      {/each}
    </nav>
    <div class="p-3 border-t border-gray-800">
      <a href="/logout" class="flex items-center gap-2 px-3 py-2 rounded-lg text-gray-400 hover:bg-gray-800 hover:text-white text-sm transition-colors">
        Sign out
      </a>
    </div>
  </aside>

  <!-- Main content -->
  <main class="flex-1 p-8 overflow-auto">
    {@render children()}
  </main>
</div>
