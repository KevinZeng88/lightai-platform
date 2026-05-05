import type { Ref } from 'vue'
import { fetchModelInstance, fetchModelInstances } from '../../api'
import type { ModelInstance } from '../../types'

export function useInstanceRefresh(instances: Ref<ModelInstance[]>) {
  let periodicTimer: ReturnType<typeof setInterval> | null = null

  function replaceInstance(updated: ModelInstance) {
    const idx = instances.value.findIndex((inst) => inst.id === updated.id)
    if (idx !== -1) instances.value[idx] = updated
  }

  async function refreshSingleInstance(id: string) {
    try {
      const updated = await fetchModelInstance(id)
      replaceInstance(updated)
    } catch {
      // keep current state on transient errors
    }
  }

  function startPeriodicRefresh() {
    if (periodicTimer) return
    periodicTimer = setInterval(async () => {
      const active = instances.value.filter((inst) =>
        ['starting', 'stopping', 'running'].includes(inst.status)
      )
      if (active.length === 0) return
      try {
        const list = await fetchModelInstances()
        for (const updated of list) {
          replaceInstance(updated)
        }
      } catch {
        // silent on transient errors
      }
    }, 15_000)
  }

  function stopPeriodicRefresh() {
    if (periodicTimer) {
      clearInterval(periodicTimer)
      periodicTimer = null
    }
  }

  return { replaceInstance, refreshSingleInstance, startPeriodicRefresh, stopPeriodicRefresh }
}
