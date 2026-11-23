import type { ListTasksReq, TaskSummary, TaskSummaryPage } from 'buckyos'

type ReadPage = (request: ListTasksReq) => Promise<TaskSummaryPage>

/** The cursor belongs to the storage partitions, so ACL-filtered pages can be empty. */
export class TaskHistory {
  private rows = new Map<string, TaskSummary>()
  private newestCreatedAt: number | undefined
  private recent: { after?: number; newest?: number; cursor?: string } | null = null
  private historyCursor: string | undefined
  private unchecked: Set<string> | null = null
  private initialScan = true

  private readonly readPage: ReadPage

  constructor(readPage: ReadPage) { this.readPage = readPage }

  get summaries(): TaskSummary[] { return [...this.rows.values()] }

  remove(taskId: string) {
    this.rows.delete(taskId)
    this.unchecked?.delete(taskId)
  }

  remember(summary: TaskSummary) {
    const old = this.rows.get(summary.task_id)
    if (!old || old.revision <= summary.revision) {
      if (summary.archived_at != null) this.rows.delete(summary.task_id)
      else this.rows.set(summary.task_id, summary)
    }
  }

  private merge(page: TaskSummaryPage) {
    for (const item of page.tasks) {
      this.remember(item)
      this.unchecked?.delete(item.task_id)
    }
  }

  private async page(cursor?: string, createdAfter?: number) {
    const request = { limit: 100, newest_first: true, cursor, created_after: createdAfter }
    const result = await this.readPage(request)
    if (cursor && result.next_cursor === cursor) throw new Error('TaskManager cursor did not advance')
    return result
  }

  private async refreshRecent(publish: () => void, stopped: () => boolean) {
    // Hold the inclusive lower bound for the lifetime of this query. A retry or
    // an ACL-empty page must not advance it past tasks we have not visited.
    const scan = this.recent ??= { after: this.newestCreatedAt, newest: this.newestCreatedAt }
    if (scan.cursor) {
      // A large burst or a sparse ACL can take several polls to traverse.
      // Show the newest arrivals while continuing the outstanding query.
      const head = await this.page(undefined, scan.newest)
      if (stopped()) return
      this.merge(head)
      if (head.tasks.length) publish()
    }
    for (let i = 0; i < 2; i++) {
      const page = await this.page(scan.cursor, scan.after)
      if (stopped()) return
      this.merge(page)
      for (const row of page.tasks) scan.newest = Math.max(scan.newest ?? 0, row.created_at)
      scan.cursor = page.next_cursor
      if (page.tasks.length) publish()
      if (!scan.cursor) {
        this.newestCreatedAt = scan.newest
        this.recent = null
        break
      }
    }
  }

  async refresh(publish: () => void, stopped: () => boolean) {
    if (!this.initialScan) {
      await this.refreshRecent(publish, stopped)
      if (stopped()) return
    }

    // First visit streams all history once. Later polls check one historical page
    // alongside new arrivals. A completed pass drops archived or no-longer-visible
    // tasks without refetching the whole archive every three seconds.
    const unchecked = this.unchecked ??= new Set(this.rows.keys())
    do {
      const page = await this.page(this.historyCursor)
      if (stopped()) return
      this.merge(page)
      if (this.initialScan) {
        for (const row of page.tasks) this.newestCreatedAt = Math.max(this.newestCreatedAt ?? 0, row.created_at)
      }
      this.historyCursor = page.next_cursor
      if (!this.historyCursor) {
        for (const id of unchecked) this.rows.delete(id)
        this.unchecked = null
        this.initialScan = false
      }
      if (page.tasks.length || !this.historyCursor) publish()
    } while (this.initialScan)
  }
}
