/**
 * test_list_models.ts — 打印 AICC 当前的模型目录树
 *
 * 调用 aicc 服务的 `models.list` RPC（在 main.rs 中由 AiccHttpServer 直接路由），
 * 拿到两份数据：
 *   - providers: 每个 provider 实例的 inventory（含 exact_model / logical_mounts）
 *   - directory: 聚合后的逻辑路径 -> 模型映射
 *
 * 然后按 `.` 分段把 directory 渲染成 ascii 目录树。
 */

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type ModelEntry = {
  exact_model: string;
  provider_model_id: string;
  api_types: string[];
  logical_mounts: string[];
  health?: string;
  quota?: string;
};

type ProviderEntry = {
  provider_instance_name: string;
  provider_driver: string;
  provider_type?: string;
  version?: string | null;
  inventory_revision?: string | null;
  models: ModelEntry[];
};

type DirectoryItem = { target: string; weight: number };
type DirectoryItems = Record<string, DirectoryItem>;
type Directory = Record<string, DirectoryItems>;

type AliasEntry = {
  capability: string;
  alias: string;
  provider_type: string;
  provider_model: string;
  tenant_id?: string;
};

type SessionItem = { target: string; weight?: number };
type SessionItems = Record<string, SessionItem>;

type SessionLogicalNode = {
  children?: Record<string, SessionLogicalNode>;
  items?: SessionItems | null;
  item_overrides?: Record<string, SessionItem> | null;
  exact_model_weights?: Record<string, number>;
  fallback?: unknown;
  policy?: unknown;
};

type SessionConfig = {
  logical_tree?: Record<string, SessionLogicalNode>;
  global_exact_model_weights?: Record<string, number>;
  policy?: unknown;
  revision?: string | null;
  ttl_seconds?: number | null;
};

type ModelsListResponse = {
  providers: ProviderEntry[];
  directory: Directory;
  aliases: AliasEntry[];
  session_config: SessionConfig;
};

type AliasLeaf = {
  capability: string;
  provider_type: string;
  provider_model: string;
  tenant_id?: string;
};

type SessionLeaf = {
  name: string;
  target: string;
  weight: number;
  source: "items" | "override";
};

type TreeNode = {
  children: Map<string, TreeNode>;
  items: DirectoryItems | null;
  aliases: AliasLeaf[];
  sessionItems: SessionLeaf[];
};

function newNode(): TreeNode {
  return { children: new Map(), items: null, aliases: [], sessionItems: [] };
}

function flattenSessionTree(
  prefix: string,
  node: SessionLogicalNode,
  sink: Map<string, SessionLeaf[]>,
): void {
  const leaves: SessionLeaf[] = [];
  if (node.items) {
    for (const [name, item] of Object.entries(node.items)) {
      leaves.push({
        name,
        target: item.target,
        weight: item.weight ?? 1.0,
        source: "items",
      });
    }
  }
  if (node.item_overrides) {
    for (const [name, item] of Object.entries(node.item_overrides)) {
      leaves.push({
        name,
        target: item.target ?? name,
        weight: item.weight ?? 1.0,
        source: "override",
      });
    }
  }
  if (leaves.length > 0) {
    const list = sink.get(prefix) ?? [];
    list.push(...leaves);
    sink.set(prefix, list);
  }
  if (node.children) {
    for (const [childName, child] of Object.entries(node.children)) {
      const childPath = prefix ? `${prefix}.${childName}` : childName;
      flattenSessionTree(childPath, child, sink);
    }
  }
}

function descend(root: TreeNode, path: string): TreeNode {
  const segments = path.split(".").filter((segment) => segment.length > 0);
  let cursor = root;
  for (const segment of segments) {
    let next = cursor.children.get(segment);
    if (!next) {
      next = newNode();
      cursor.children.set(segment, next);
    }
    cursor = next;
  }
  return cursor;
}

function buildTree(
  directory: Directory,
  aliases: AliasEntry[],
  sessionLeaves: Map<string, SessionLeaf[]>,
): TreeNode {
  const root = newNode();
  const paths = Object.keys(directory).sort();
  for (const path of paths) {
    const node = descend(root, path);
    node.items = directory[path];
  }
  for (const alias of aliases) {
    const node = descend(root, alias.alias);
    node.aliases.push({
      capability: alias.capability,
      provider_type: alias.provider_type,
      provider_model: alias.provider_model,
      tenant_id: alias.tenant_id,
    });
  }
  for (const [path, leaves] of sessionLeaves.entries()) {
    const node = descend(root, path);
    node.sessionItems.push(...leaves);
  }
  return root;
}

function renderTree(node: TreeNode, prefix: string, lines: string[]): void {
  const childKeys = Array.from(node.children.keys()).sort();
  childKeys.forEach((key, index) => {
    const isLast = index === childKeys.length - 1;
    const branch = isLast ? "└── " : "├── ";
    const child = node.children.get(key)!;
    lines.push(`${prefix}${branch}${key}`);
    const nextPrefix = `${prefix}${isLast ? "    " : "│   "}`;

    type Leaf = { label: string };
    const leaves: Leaf[] = [];
    if (child.items) {
      for (const itemKey of Object.keys(child.items).sort()) {
        const item = child.items[itemKey];
        const weight = item.weight === 1 ? "" : `  (w=${item.weight})`;
        leaves.push({ label: `${item.target}${weight}` });
      }
    }
    const sortedSession = [...child.sessionItems].sort((left, right) =>
      left.name.localeCompare(right.name)
    );
    for (const session of sortedSession) {
      const weight = session.weight === 1 ? "" : `  (w=${session.weight})`;
      const tag = session.source === "override" ? "override" : "ref";
      leaves.push({
        label: `[${tag}: ${session.name} → ${session.target}]${weight}`,
      });
    }
    for (const alias of child.aliases) {
      const tenant = alias.tenant_id ? `  tenant=${alias.tenant_id}` : "";
      leaves.push({
        label:
          `[alias→ ${alias.provider_type}/${alias.provider_model}] (${alias.capability})${tenant}`,
      });
    }
    leaves.forEach((leaf, leafIndex) => {
      const leafIsLast =
        leafIndex === leaves.length - 1 && child.children.size === 0;
      const leafBranch = leafIsLast ? "└── " : "├── ";
      lines.push(`${nextPrefix}${leafBranch}${leaf.label}`);
    });
    renderTree(child, nextPrefix, lines);
  });
}

function renderProviders(providers: ProviderEntry[]): string[] {
  const sorted = [...providers].sort((left, right) =>
    left.provider_instance_name.localeCompare(right.provider_instance_name)
  );
  const lines: string[] = [];
  for (const provider of sorted) {
    const driver = provider.provider_driver || "<unknown>";
    const typ = provider.provider_type ?? "<unknown>";
    lines.push(`${provider.provider_instance_name}  [${driver} / ${typ}]`);
    const sortedModels = [...provider.models].sort((left, right) =>
      left.exact_model.localeCompare(right.exact_model)
    );
    sortedModels.forEach((model, index) => {
      const isLast = index === sortedModels.length - 1;
      const branch = isLast ? "└── " : "├── ";
      const apis = model.api_types.length > 0
        ? `  api=[${model.api_types.join(",")}]`
        : "";
      const health = model.health ? `  health=${model.health}` : "";
      lines.push(`  ${branch}${model.exact_model}${apis}${health}`);
      const mountPrefix = `  ${isLast ? "    " : "│   "}`;
      const mounts = model.logical_mounts.length > 0
        ? model.logical_mounts.join(", ")
        : "<none>";
      lines.push(`${mountPrefix}mounts: ${mounts}`);
    });
  }
  return lines;
}

async function main(): Promise<void> {
  const { buckyos, userId, zoneHost } = await initTestRuntime();
  const aiccRpc = buckyos.getServiceRpcClient("aicc") as RpcClient;

  const raw = await aiccRpc.call("models.list", {});
  if (!raw || typeof raw !== "object") {
    throw new Error(`unexpected models.list response: ${JSON.stringify(raw)}`);
  }
  const result = raw as ModelsListResponse;
  const providers = Array.isArray(result.providers) ? result.providers : [];
  const directory = result.directory ?? {};
  const aliases = Array.isArray(result.aliases) ? result.aliases : [];
  const sessionConfig = result.session_config ?? {};
  const sessionLeaves = new Map<string, SessionLeaf[]>();
  if (sessionConfig.logical_tree) {
    for (const [name, node] of Object.entries(sessionConfig.logical_tree)) {
      flattenSessionTree(name, node, sessionLeaves);
    }
  }

  console.log("=== AICC Model Directory ===");
  console.log(`Zone: ${zoneHost}`);
  console.log(`User ID: ${userId}`);
  console.log(`Providers: ${providers.length}`);
  console.log(`Inventory mounts: ${Object.keys(directory).length}`);
  console.log(`Catalog aliases: ${aliases.length}`);
  console.log(`Session-config nodes: ${sessionLeaves.size}`);
  console.log(`Session-config revision: ${sessionConfig.revision ?? "<none>"}`);

  console.log("\n--- Providers ---");
  if (providers.length === 0) {
    console.log("(no providers registered)");
  } else {
    for (const line of renderProviders(providers)) {
      console.log(line);
    }
  }

  console.log("\n--- Catalog aliases ---");
  if (aliases.length === 0) {
    console.log("(none)");
  } else {
    const grouped = new Map<string, AliasEntry[]>();
    for (const alias of aliases) {
      const key = alias.alias;
      const list = grouped.get(key) ?? [];
      list.push(alias);
      grouped.set(key, list);
    }
    for (const aliasName of Array.from(grouped.keys()).sort()) {
      console.log(aliasName);
      const entries = grouped.get(aliasName)!;
      entries.sort((left, right) =>
        left.provider_type.localeCompare(right.provider_type) ||
        left.provider_model.localeCompare(right.provider_model)
      );
      entries.forEach((entry, index) => {
        const isLast = index === entries.length - 1;
        const branch = isLast ? "└── " : "├── ";
        const tenant = entry.tenant_id ? `  tenant=${entry.tenant_id}` : "";
        console.log(
          `  ${branch}${entry.provider_type}/${entry.provider_model}  (${entry.capability})${tenant}`,
        );
      });
    }
  }

  console.log("\n--- Logical directory tree ---");
  console.log(
    "(legend: bare 'name@provider' = inventory mount; '[ref: name → llm.xxx] (w=...)' = SessionConfig 2-level item; '[alias→ ...]' = ModelCatalog mapping)",
  );
  const totalEntries = Object.keys(directory).length + aliases.length +
    sessionLeaves.size;
  if (totalEntries === 0) {
    console.log("(empty)");
  } else {
    const tree = buildTree(directory, aliases, sessionLeaves);
    const lines: string[] = [];
    renderTree(tree, "", lines);
    for (const line of lines) {
      console.log(line);
    }
  }

  buckyos.logout(false);
}

main().catch((error) => {
  console.error("AICC list models test failed");
  console.error(error);
  Deno.exit(1);
});
