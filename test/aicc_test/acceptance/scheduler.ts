export type ProviderLimits = {
  maxConcurrency: number;
  minIntervalMs: number;
};

export type ScheduledItem = { provider_driver: string };

class Semaphore {
  private active = 0;
  private readonly waiters: Array<() => void> = [];
  private readonly capacity: number;

  constructor(capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new Error("semaphore capacity must be a positive integer");
    }
    this.capacity = capacity;
  }

  async acquire(): Promise<() => void> {
    if (this.active >= this.capacity) {
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
    this.active += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.active -= 1;
      this.waiters.shift()?.();
    };
  }
}

class Mutex {
  private tail: Promise<void> = Promise.resolve();

  async acquire(): Promise<() => void> {
    let unlock!: () => void;
    const current = new Promise<void>((resolve) => { unlock = resolve; });
    const previous = this.tail;
    this.tail = previous.then(() => current);
    await previous;
    return unlock;
  }
}

function positiveInteger(value: number, name: string): number {
  if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be a positive integer`);
  return value;
}

function nonNegative(value: number, name: string): number {
  if (!Number.isFinite(value) || value < 0) throw new Error(`${name} must be non-negative`);
  return value;
}

export class ProviderScheduler {
  private readonly global: Semaphore;
  private readonly providerSemaphores = new Map<string, Semaphore>();
  private readonly providerStartLocks = new Map<string, Mutex>();
  private readonly lastStartedAt = new Map<string, number>();
  private readonly defaultLimits: ProviderLimits;
  private readonly overrides: Readonly<Record<string, Partial<ProviderLimits>>>;

  constructor(
    globalConcurrency: number,
    defaultLimits: ProviderLimits,
    overrides: Readonly<Record<string, Partial<ProviderLimits>>> = {},
  ) {
    this.global = new Semaphore(positiveInteger(globalConcurrency, "global concurrency"));
    this.validateLimits(defaultLimits, "default provider limits");
    for (const [provider, limits] of Object.entries(overrides)) {
      this.validateLimits({ ...defaultLimits, ...limits }, `limits for ${provider}`);
    }
    this.defaultLimits = { ...defaultLimits };
    this.overrides = overrides;
  }

  private validateLimits(limits: ProviderLimits, name: string): void {
    positiveInteger(limits.maxConcurrency, `${name}.maxConcurrency`);
    nonNegative(limits.minIntervalMs, `${name}.minIntervalMs`);
  }

  private limits(provider: string): ProviderLimits {
    return { ...this.defaultLimits, ...this.overrides[provider] };
  }

  private providerSemaphore(provider: string): Semaphore {
    let value = this.providerSemaphores.get(provider);
    if (!value) {
      value = new Semaphore(this.limits(provider).maxConcurrency);
      this.providerSemaphores.set(provider, value);
    }
    return value;
  }

  private providerStartLock(provider: string): Mutex {
    let value = this.providerStartLocks.get(provider);
    if (!value) {
      value = new Mutex();
      this.providerStartLocks.set(provider, value);
    }
    return value;
  }

  async execute<T>(provider: string, operation: () => Promise<T>): Promise<T> {
    if (!provider.trim()) throw new Error("provider_driver is required");
    const releaseProvider = await this.providerSemaphore(provider).acquire();
    let releaseGlobal: (() => void) | undefined;
    try {
      const releaseStartLock = await this.providerStartLock(provider).acquire();
      try {
        const waitMs = Math.max(
          0,
          (this.lastStartedAt.get(provider) ?? 0) + this.limits(provider).minIntervalMs - Date.now(),
        );
        if (waitMs > 0) await new Promise((resolve) => setTimeout(resolve, waitMs));
        releaseGlobal = await this.global.acquire();
        this.lastStartedAt.set(provider, Date.now());
      } finally {
        releaseStartLock();
      }
      return await operation();
    } finally {
      releaseGlobal?.();
      releaseProvider();
    }
  }

  run<TItem extends ScheduledItem, TResult>(
    items: readonly TItem[],
    operation: (item: TItem, index: number) => Promise<TResult>,
  ): Promise<TResult[]> {
    return Promise.all(items.map((item, index) =>
      this.execute(item.provider_driver, () => operation(item, index))
    ));
  }
}
