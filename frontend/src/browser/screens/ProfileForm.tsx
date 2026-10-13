import { useMemo, useRef, useState, type FormEvent, type MouseEvent } from 'react';
import { Lock } from 'lucide-react';
import { Input } from '@/components/silicon-ui/input/input';
import { MorphSelect, type MorphSelectOption } from '@/components/silicon-ui/morph-select/morph-select';
import { TagInput } from '@/components/silicon-ui/tag-input/tag-input';
import { Textarea } from '@/components/silicon-ui/textarea/textarea';
import { splitAccess } from '@/lib/format';
import type { Profile } from '@/lib/types';
import { ButtonRow, FormStack, PageHeader, Panel, SubmitButton, TaskButton } from '../ui';
import { useWorkspace } from '../workspace/WorkspaceContext';
import styles from './ProfileForm.module.css';

/**
 * Access tags exactly as `saveProfile` will send them: a pasted "a, b" splits on commas (like the former text field), each tag is
 * trimmed, empty tags are dropped, and repeats are removed (TagInput keys tags by text and already ignores repeats in any case).
 */
function cleanAccess(tags: readonly string[]) {
  const seen = new Set<string>();
  return tags.flatMap(splitAccess).filter(tag => {
    const key = tag.toLowerCase();
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/**
 * Pressing Save keeps focus where it is. Otherwise the press blurs the access field, which commits its draft as a tag and grows
 * the field on a spring, moving the button out from under the pointer before mouseup, so the click never lands.
 */
function keepFocus(event: MouseEvent<HTMLButtonElement>) { event.preventDefault(); }

/** New profile (view `new-profile`, no `profile`) or edit profile (view `edit-profile`, `profile` set). */
export function ProfileForm({ profile }: { profile?: Profile }) {
  // A fresh form per profile, so prefilled fields and access tags never carry over between new and edit.
  return <ProfileFormBody key={profile?.id ?? 'new'} profile={profile} />;
}

function ProfileFormBody({ profile }: { profile?: Profile }) {
  const { busy, locations, submit, saveProfile, retireProfile, navigate } = useWorkspace();
  const [access, setAccess] = useState(() => cleanAccess(profile?.access ?? []));
  const accessField = useRef<HTMLFieldSetElement>(null);
  /** Saves the tags plus a UUID still being typed: TagInput commits a draft only on Enter or blur, and Save takes no focus. */
  const save = (event: FormEvent<HTMLFormElement>) => {
    const draft = accessField.current?.querySelector('input')?.value ?? '';
    submit(data => { data.set('access', cleanAccess([...access, draft]).join(',')); return saveProfile(data); })(event);
  };
  const places = useMemo<MorphSelectOption[]>(() => locations.map(place => ({ value: place.code, label: place.name, meta: place.code.toUpperCase(), keywords: place.code })), [locations]);
  return <div className={styles.screen}>
    <PageHeader
      eyebrow="Persistent identities"
      title={profile ? 'Edit profile' : 'New profile'}
      subtitle="Your profile’s location stays fixed after creation."
      actions={<TaskButton task={() => navigate('profiles')}>Back to profiles</TaskButton>}
    />
    <Panel narrow>
      <FormStack onSubmit={save}>
        <Input name="name" label="Profile name" required maxLength={100} defaultValue={profile?.name ?? ''} placeholder="Research workspace" disabled={busy} />
        {/* MorphSelect has no `required`; selecting the first location by default matches the former native select. */}
        {profile
          ? <dl className={styles.fixed}>
            <dt className={styles.fixedTerm}>Location</dt>
            <dd className={styles.fixedValue}><Lock size={14} strokeWidth={1.75} aria-hidden="true" />{profile.location.name}</dd>
          </dl>
          : <MorphSelect name="location" label="Location" items={places} searchable="auto" defaultValue={locations[0]?.code}
            placeholder={locations.length ? 'Choose a location' : 'No locations available'} disabled={busy || !locations.length} />}
        {/* TagInput has no `disabled`; a bare fieldset disables its field and remove buttons while busy. */}
        <fieldset ref={accessField} className={styles.access} disabled={busy}>
          <TagInput label="Additional access" placeholder="Account UUID" value={access} onValueChange={next => setAccess(cleanAccess(next))}
            description="Press Enter after each account UUID. Your account always keeps owner access." />
        </fieldset>
        {/* TagInput has no `name`: saveProfile reads `access` as comma-separated UUIDs, as before (`save` adds a pending draft). */}
        <input type="hidden" name="access" value={access.join(',')} />
        <ButtonRow><SubmitButton disabled={!profile && !locations.length} onMouseDown={keepFocus}>Save profile</SubmitButton></ButtonRow>
      </FormStack>
    </Panel>
    {profile && <Panel narrow tone="subtle" title="Retire profile" description="Retired profiles cannot start new sessions.">
      <FormStack onSubmit={submit(retireProfile)}>
        <Textarea name="note" label="Reason" required maxLength={4000} disabled={busy} />
        <ButtonRow><SubmitButton variant="danger">Retire profile</SubmitButton></ButtonRow>
      </FormStack>
    </Panel>}
  </div>;
}
