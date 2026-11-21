import { createContext } from 'react'
import type { RenderContext } from '../blocks/registry'

export const TableBlockContext = createContext<RenderContext | null>(null)
