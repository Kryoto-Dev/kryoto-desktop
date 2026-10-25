import { useMemo, useState } from 'react'
import { ProfileLink, TeamName } from '@/ui/TeamName'
import { Crown, LogOut, Shield, UserMinus } from 'lucide-react'
import { errorText } from '@/lib/bridge'
import { chatGroupAdd, chatGroupCreate, chatGroupRemove, chatGroupRename, here, type GroupView } from '@/lib/chat'
import { friendName, type FriendFace } from '@/hooks/useFriends'
import { Button, Caption, Check, inputCls, Modal } from '@/ui'

/** Up to 32 people in a group, you included. */
const MAX_GROUP = 32

function PickFriends({
  friends,
  picked,
  onToggle,
  exclude = [],
}: {
  friends: (FriendFace & { nickname?: string | null })[]
  picked: Set<string>
  onToggle: (id: string) => void
  exclude?: string[]
}) {
  const [q, setQ] = useState('')
  const list = useMemo(() => {
    const n = q.trim().toLowerCase()
    return friends
      .filter((f) => !exclude.includes(f.id))
      .filter((f) => !n || [f.nickname, f.displayName, f.username].some((v) => v?.toLowerCase().includes(n)))
  }, [friends, q, exclude])
  return (
    <div className="grid gap-2">
      <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search friends" aria-label="Search friends" className={inputCls} />
      <ul className="kryo-radius grid max-h-60 gap-1 overflow-auto border border-border p-2">
        {list.length === 0 ? <li className="p-2 text-xs text-muted-foreground">Nobody to add.</li> : null}
        {list.map((f) => (
          <li key={f.id}>
            <Check checked={picked.has(f.id)} onChange={() => onToggle(f.id)} label={<ProfileLink username={f.username} className="hover:underline focus-visible:outline-2 focus-visible:outline-primary"><TeamName username={f.username}>{friendName(f)}</TeamName></ProfileLink>} />
          </li>
        ))}
      </ul>
    </div>
  )
}

/** Start a group: a name and some friends. */
export function GroupCreateDialog({
  friends,
  onClose,
  onCreated,
}: {
  friends: (FriendFace & { nickname?: string | null })[]
  onClose: () => void
  onCreated: (g: GroupView) => void
}) {
  const [name, setName] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else if (n.size < MAX_GROUP - 1) n.add(id)
      return n
    })
  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      onCreated(await chatGroupCreate(name.trim(), [...picked]))
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal
      title="New group"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={busy || !name.trim() || picked.size === 0} onClick={() => void create()}>
            Create
          </Button>
        </>
      }
    >
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value.slice(0, 64))}
        placeholder="Group name"
        aria-label="Group name"
        className={inputCls}
      />
      <Caption>Who ({picked.size + 1} of {MAX_GROUP}, you included)</Caption>
      <PickFriends friends={friends} picked={picked} onToggle={toggle} />
      <p className="text-[11px] text-muted-foreground">
        End-to-end encrypted like your other chats. Kryoto knows who is in the group, never its name or what is said.
      </p>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}

/** Who is in a group; rename, add, remove (owner and admins), leave. */
export function GroupMembersDialog({
  group,
  myId,
  friends,
  nameOf,
  usernameOf = () => null,
  onClose,
  onLeft,
}: {
  group: GroupView
  myId: string
  friends: (FriendFace & { nickname?: string | null })[]
  nameOf: (id: string) => string
  usernameOf?: (id: string) => string | null
  onClose: () => void
  onLeft: () => void
}) {
  const canManage = group.myRole === 'owner' || group.myRole === 'admin'
  const [name, setName] = useState(group.name)
  const [adding, setAdding] = useState(false)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [leaving, setLeaving] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const act = async (work: () => Promise<unknown>, after?: () => void) => {
    setBusy(true)
    setError(null)
    try {
      await work()
      after?.()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  const roleIcon = (role: string) =>
    role === 'owner' ? <Crown className="size-3 text-primary" aria-label="Owner" /> : role === 'admin' ? <Shield className="size-3" aria-label="Admin" /> : null

  if (leaving) {
    return (
      <Modal
        title={`Leave ${group.name}?`}
        onClose={() => setLeaving(false)}
        footer={
          <>
            <Button onClick={() => setLeaving(false)}>Cancel</Button>
            <Button variant="danger" disabled={busy} onClick={() => void act(() => chatGroupRemove(group.id, myId), onLeft)}>
              Leave
            </Button>
          </>
        }
      >
        <p className="text-sm text-foreground">You stop getting its messages. What was said stays on {here()}.</p>
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </Modal>
    )
  }

  return (
    <Modal title={group.name} onClose={onClose}>
      {canManage ? (
        <div className="flex gap-2">
          <input value={name} onChange={(e) => setName(e.target.value.slice(0, 64))} aria-label="Group name" className={inputCls} />
          <Button size="sm" disabled={busy || !name.trim() || name.trim() === group.name} onClick={() => void act(() => chatGroupRename(group.id, name.trim()))}>
            Rename
          </Button>
        </div>
      ) : null}
      <Caption>Members ({group.members.length})</Caption>
      <ul className="grid gap-1">
        {group.members.map((m) => (
          <li key={m.userId} className="kryo-radius flex items-center justify-between gap-2 border border-border px-3 py-1.5 text-xs text-foreground">
            <span className="flex min-w-0 items-center gap-2">
              <ProfileLink username={m.userId === myId ? null : usernameOf(m.userId)} className="truncate hover:underline focus-visible:outline-2 focus-visible:outline-primary"><TeamName username={m.userId === myId ? null : usernameOf(m.userId)}>{m.userId === myId ? 'You' : nameOf(m.userId)}</TeamName></ProfileLink>
              {roleIcon(m.role)}
            </span>
            {canManage && m.userId !== myId && (group.myRole === 'owner' || m.role === 'member') ? (
              <Button size="sm" variant="danger" disabled={busy} onClick={() => void act(() => chatGroupRemove(group.id, m.userId))} aria-label={`Remove ${nameOf(m.userId)}`}>
                <UserMinus className="size-3" aria-hidden />
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      {canManage ? (
        adding ? (
          <div className="grid gap-2">
            <PickFriends
              friends={friends}
              picked={picked}
              exclude={group.members.map((m) => m.userId)}
              onToggle={(id) =>
                setPicked((s) => {
                  const n = new Set(s)
                  if (n.has(id)) n.delete(id)
                  else if (group.members.length + n.size < MAX_GROUP) n.add(id)
                  return n
                })
              }
            />
            <div className="flex gap-2">
              <Button size="sm" onClick={() => setAdding(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || picked.size === 0}
                onClick={() =>
                  void act(
                    () => chatGroupAdd(group.id, [...picked]),
                    () => {
                      setPicked(new Set())
                      setAdding(false)
                    },
                  )
                }
              >
                Add {picked.size || ''}
              </Button>
            </div>
          </div>
        ) : (
          <div>
            <Button size="sm" onClick={() => setAdding(true)} disabled={group.members.length >= MAX_GROUP}>
              Add friends
            </Button>
          </div>
        )
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      <div className="border-t border-border pt-3">
        <Button size="sm" variant="danger" onClick={() => setLeaving(true)}>
          <LogOut className="size-3" aria-hidden /> Leave group
        </Button>
      </div>
    </Modal>
  )
}
