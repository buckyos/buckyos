import { useSearchParams } from 'react-router-dom'
import { ShieldCheck } from 'lucide-react'

// Example login-optional internal page.
//
// Reachable at /userprofile?user=<id> without a signed-in account thanks to the
// public-route gate in main.tsx / publicRoutes.ts. For now the profile is a
// temporary placeholder derived from the `user` query param; wiring it to a real
// public-profile API can come later.

interface TempProfile {
  id: string
  name: string
  bio: string
  postCount: number
  followerCount: number
  followingCount: number
}

function buildTempProfile(userId: string): TempProfile {
  const name = userId.charAt(0).toUpperCase() + userId.slice(1)
  return {
    id: userId,
    name,
    bio: `Public profile of ${name} (temporary placeholder)`,
    postCount: 0,
    followerCount: 0,
    followingCount: 0,
  }
}

export function UserProfileRoute() {
  const [searchParams] = useSearchParams()
  const userId = searchParams.get('user') ?? 'guest'
  const profile = buildTempProfile(userId)

  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] p-0 md:p-5">
      <div
        className="mx-auto w-full overflow-hidden md:max-w-[720px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background: 'var(--cp-surface)',
        }}
      >
        {/* Cover area */}
        <div
          className="relative h-32 w-full md:h-48"
          style={{
            background:
              'linear-gradient(135deg, color-mix(in srgb, var(--cp-accent) 30%, var(--cp-surface)), color-mix(in srgb, var(--cp-accent-soft) 40%, var(--cp-surface-2)))',
          }}
        />

        {/* Profile info */}
        <div className="relative px-4 pb-6">
          <div
            className="-mt-10 flex h-20 w-20 items-center justify-center rounded-full border-4 text-2xl font-bold md:-mt-12 md:h-24 md:w-24"
            style={{
              background:
                'color-mix(in srgb, var(--cp-accent) 20%, var(--cp-surface))',
              borderColor: 'var(--cp-bg)',
              color: 'var(--cp-accent)',
            }}
          >
            {profile.name.charAt(0)}
          </div>

          <div className="mt-3">
            <div className="flex items-center gap-2">
              <h2
                className="text-xl font-bold"
                style={{ color: 'var(--cp-text)' }}
              >
                {profile.name}
              </h2>
              <ShieldCheck size={18} style={{ color: 'var(--cp-success)' }} />
            </div>
            <p
              className="mt-1 text-sm"
              style={{
                color: 'color-mix(in srgb, var(--cp-text) 75%, transparent)',
              }}
            >
              {profile.bio}
            </p>
            <div
              className="mt-2 flex items-center gap-4 text-sm"
              style={{ color: 'var(--cp-muted)' }}
            >
              <span>
                <strong style={{ color: 'var(--cp-text)' }}>
                  {profile.postCount}
                </strong>{' '}
                posts
              </span>
              <span>
                <strong style={{ color: 'var(--cp-text)' }}>
                  {profile.followerCount}
                </strong>{' '}
                followers
              </span>
              <span>
                <strong style={{ color: 'var(--cp-text)' }}>
                  {profile.followingCount}
                </strong>{' '}
                following
              </span>
            </div>
          </div>
        </div>
      </div>
    </main>
  )
}

export default UserProfileRoute
