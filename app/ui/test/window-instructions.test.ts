import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, INSTRUCTIONS_PATH, NOW, STANDARD_INSTRUCTIONS, agents, click, core, feeds, instructions, instructionsFile, mount, settle, stub, text, view } from './windowFixture';
import type { InstructionsNews } from '../src/window/api';
import type { WindowPage } from '../src/window/page';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;

/** The tray opened on the Cleanup page, then the Instructions page from its row. */
const openPage = async () => {
  click($('.gear'));
  await settle();
  click($('.btray [data-sub="cleanup"]'));
  await settle();
  click($('.btray [data-sub="instructions"]'));
  await settle();
};
const box = () => $<HTMLTextAreaElement>('.btray textarea.pta')!;
const title = () => text($('.btray .ihead .stx b'));
const line = () => $<HTMLElement>('.btray .ihead .stx small')!;
const done = () => $<HTMLElement>('.btray .idone')!;
/** `cue://instructions` as the core sends it: the file read again, by default with the before and after being made. */
const news = (over: Partial<InstructionsNews>): InstructionsNews => ({
  text: STANDARD_INSTRUCTIONS, edited: false, broken: false, path: INSTRUCTIONS_PATH, open: true, sample: null, check: 'checking', ...over,
});
const check = () => $<HTMLElement>('.btray .icheck');
const menuButtons = () => $$('.btray .pmenu button').filter((b) => !b.closest('[hidden]'));

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub(view({ refineWhere: 'agent', refineAgent: 'codex' }));
  core.agentsGet.mockResolvedValue(agents({ opencode: { found: true }, codex: { found: true, signedIn: true } }));
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The Instructions page', () => {
  it('page_shows_file_text_and_standard_state', async () => {
    page = await mount();
    await openPage();
    expect(text($('.btray .tph .lbl'))).toBe('Your instructions');
    expect(box().value).toBe(STANDARD_INSTRUCTIONS);
    expect(box().getAttribute('aria-label')).toBe('Instructions');
    expect(title()).toBe("CUE's standard");
    expect(text(line())).toBe('Used by your AI and by the model on this computer');
    expect(line().classList.contains('warn')).toBe(false);
    expect($('.btray [data-ireset]')).toBeNull();
    expect($('.btray .cmpr')).toBeNull();
    expect(text($('.btray .tft'))).toContain(INSTRUCTIONS_PATH);
    expect(text($('.btray .tft'))).toContain('AGENTS.md');
    // The user's own instructions: restore is offered.
    core.instructionsGet.mockResolvedValue(instructions({ text: 'Keep it short.', edited: true }));
    click($('.btray [data-tback]'));
    await settle();
    click($('.btray [data-sub="instructions"]'));
    await settle();
    expect(box().value).toBe('Keep it short.');
    expect(title()).toBe('Changed by you');
    expect(text($('.btray [data-ireset]'))).toBe('Restore standard');
  });

  it('saves_the_text_when_the_box_is_left', async () => {
    page = await mount();
    await openPage();
    box().value = 'Keep it short.';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(core.instructionsSet).toHaveBeenCalledWith('Keep it short.', STANDARD_INSTRUCTIONS);
    expect(title()).toBe('Changed by you');
    // Unchanged text is not saved again; an edit left unsaved goes in when the page is left.
    core.instructionsSet.mockClear();
    box().dispatchEvent(new Event('change', { bubbles: true }));
    box().value = 'Keep it shorter.';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    click($('.btray [data-tback]'));
    await settle();
    expect(core.instructionsSet).toHaveBeenCalledTimes(1);
    expect(core.instructionsSet).toHaveBeenCalledWith('Keep it shorter.', 'Keep it short.');
  });

  it('edit_with_ai_menu_has_open_and_copy', async () => {
    page = await mount();
    await openPage();
    const ai = () => $<HTMLElement>('.btray [data-iai]')!;
    expect(text(ai())).toBe('Edit with AI');
    expect(ai().getAttribute('aria-expanded')).toBe('false');
    expect(menuButtons()).toHaveLength(0);
    click(ai());
    await settle();
    expect(ai().getAttribute('aria-expanded')).toBe('true');
    expect(menuButtons().map((b) => text(b.querySelector('b')))).toEqual(['Open in Codex', 'Copy the task']);
    expect(text(menuButtons()[0].querySelector('small'))).toBe('Codex starts in the instructions folder and asks what to change');
    click($('.btray [data-irun]'));
    await settle();
    expect(core.instructionsOpen).toHaveBeenCalledWith('codex');
    expect(menuButtons()).toHaveLength(0);
    expect(text(done())).toBe('Codex is open. When the AI saves the file, CUE takes it by itself.');
    // It opened without the task (the core copied it): the page says to paste it.
    core.instructionsOpen.mockResolvedValue('taskCopied');
    click(ai());
    click($('.btray [data-irun]'));
    await settle();
    expect(text(done())).toBe('Codex is open, the task copied: paste it there.');
    // It did not open: amber, with "!".
    core.instructionsOpen.mockRejectedValue('notFound');
    click(ai());
    click($('.btray [data-irun]'));
    await settle();
    expect(done().classList.contains('warn')).toBe(true);
    expect(text(done())).toBe("!Codex didn't open");
  });

  it('opens_the_first_agent_found_when_none_is_chosen_and_offers_only_the_copy_without_one', async () => {
    stub(view({ refineWhere: 'local' }));
    core.agentsGet.mockResolvedValue(agents({ codex: { found: true, signedIn: true } }));
    page = await mount();
    await openPage();
    click($('.btray [data-iai]'));
    await settle();
    expect(menuButtons().map((b) => text(b.querySelector('b')))).toEqual(['Open in Codex', 'Copy the task']);
    core.agentsGet.mockResolvedValue(agents());
    page.dispose();
    page = await mount();
    await openPage();
    click($('.btray [data-iai]'));
    await settle();
    expect(menuButtons().map((b) => text(b.querySelector('b')))).toEqual(['Copy the task']);
  });

  it('copy_puts_task_on_clipboard', async () => {
    page = await mount();
    await openPage();
    click($('.btray [data-iai]'));
    await settle();
    click($('.btray [data-icopy]'));
    await settle();
    expect(core.instructionsCopy).toHaveBeenCalledTimes(1);
    expect(core.instructionsOpen).not.toHaveBeenCalled();
    expect(menuButtons()).toHaveLength(0);
    expect(text(done())).toBe('Task copied. When the AI saves the file, CUE takes it by itself.');
  });

  it('reload_event_shows_before_after', async () => {
    page = await mount();
    await openPage();
    // The AI saved the file: the page shows it at once, "just now".
    feeds.instructions(news({ text: 'In Telegram: lower case, no full stop.', edited: true, broken: false, sample: null }));
    await settle();
    expect(box().value).toBe('In Telegram: lower case, no full stop.');
    expect(title()).toBe('Changed by you');
    expect(text(line())).toBe('Just now');
    expect($('.btray .cmpr')).toBeNull();
    // Then the last dictation cleaned again with them.
    const sample = { app: 'Telegram', before: 'Send the file in 5 minutes.', after: 'send the file in 5 minutes' };
    feeds.instructions(news({ text: 'In Telegram: lower case, no full stop.', edited: true, broken: false, sample, check: 'shown' }));
    await settle();
    const rows = $$('.btray .cmpr > div');
    expect(rows.map((r) => text(r.querySelector('small')))).toEqual(['Last dictation in Telegram, before', 'After']);
    expect(text(rows[0].querySelector('s'))).toBe('Send the file in 5 minutes.');
    expect(text(rows[1].querySelector('span'))).toBe('send the file in 5 minutes');
    // No app known: the label says so without one.
    feeds.instructions(news({ text: 'x', edited: true, broken: false, sample: { ...sample, app: '' }, check: 'shown' }));
    await settle();
    expect(text($('.btray .cmpr small'))).toBe('Last dictation, before');
  });

  it('a_text_being_typed_is_not_replaced_by_the_file', async () => {
    page = await mount();
    await openPage();
    box().focus();
    box().value = 'My own words';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    feeds.instructions(news({ text: 'From the AI.', edited: true, broken: false, sample: null }));
    await settle();
    expect(box().value).toBe('My own words');
    // The box was not moved: the caret is still in it.
    expect(document.activeElement).toBe(box());
  });

  it('a_focused_box_with_no_edit_takes_the_ais_text', async () => {
    page = await mount();
    await openPage();
    box().focus();
    instructionsFile({ text: 'From the AI.', edited: true });
    feeds.instructions(news({ text: 'From the AI.', edited: true }));
    await settle();
    expect(box().value).toBe('From the AI.');
    expect(document.activeElement).toBe(box());
    // A small edit then is saved over the AI's text, never over the old one.
    box().value = 'From the AI. Shorter.';
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(core.instructionsSet).toHaveBeenCalledWith('From the AI. Shorter.', 'From the AI.');
    expect($('.btray .iconf')).toBeNull();
  });

  it('an_unsaved_edit_and_an_ai_save_ask_which_to_keep', async () => {
    page = await mount();
    await openPage();
    box().focus();
    box().value = 'My own words';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    instructionsFile({ text: 'From the AI.', edited: true });
    feeds.instructions(news({ text: 'From the AI.', edited: true }));
    await settle();
    // Nothing replaced, nothing saved: the choice, in amber with "!".
    expect(box().value).toBe('My own words');
    expect(text($('.btray .iwarn'))).toBe('!The file changed while you were writing.');
    expect($$('.btray .iconf button').map((b) => text(b))).toEqual(["Take the file's", 'Keep mine']);
    // Leaving the box saves nothing over the AI's file: the choice stays.
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(core.instructionsSet).toHaveBeenCalledWith('My own words', STANDARD_INSTRUCTIONS);
    expect(await core.instructionsGet()).toMatchObject({ text: 'From the AI.' });
    expect($('.btray .iconf')).not.toBeNull();
    // "Keep mine": saved whatever the file holds.
    click($('.btray [data-imine]'));
    await settle();
    expect(core.instructionsSet).toHaveBeenLastCalledWith('My own words', null);
    expect(box().value).toBe('My own words');
    expect($('.btray .iconf')).toBeNull();
    // Another round, and "Take the file's": the box takes it, nothing is saved.
    box().value = 'Mine again';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    instructionsFile({ text: 'The AI again.', edited: true });
    feeds.instructions(news({ text: 'The AI again.', edited: true }));
    await settle();
    core.instructionsSet.mockClear();
    click($('.btray [data-itheirs]'));
    await settle();
    expect(box().value).toBe('The AI again.');
    expect($('.btray .iconf')).toBeNull();
    expect(core.instructionsSet).not.toHaveBeenCalled();
  });

  it('an_edit_refused_when_leaving_the_page_comes_back_with_the_choice', async () => {
    page = await mount();
    await openPage();
    box().value = 'My own words';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    // The AI saves before the page hears it; the page is left.
    instructionsFile({ text: 'From the AI.', edited: true });
    click($('.btray [data-tback]'));
    await settle();
    click($('.btray [data-sub="instructions"]'));
    await settle();
    expect(box().value).toBe('My own words');
    expect($('.btray .iconf')).not.toBeNull();
  });

  it('says_while_checking_and_when_the_before_and_after_comes_later', async () => {
    page = await mount();
    await openPage();
    feeds.instructions(news({ text: 'Keep it short.', edited: true, check: 'checking' }));
    await settle();
    expect(text(check())).toBe('Checking on your last dictation…');
    feeds.instructions(news({ text: 'Keep it short.', edited: true, check: 'later' }));
    await settle();
    expect(text(check())).toBe('Before and after show up after your next dictation.');
    feeds.instructions(news({ text: 'Keep it short.', edited: true, check: 'off' }));
    await settle();
    expect(check()).toBeNull();
  });

  it('a_failed_copy_says_so_and_a_folder_no_terminal_takes_offers_only_the_copy', async () => {
    core.instructionsCopy.mockRejectedValue('core');
    core.instructionsGet.mockResolvedValue(instructions({ open: false }));
    page = await mount();
    await openPage();
    click($('.btray [data-iai]'));
    await settle();
    expect(menuButtons().map((b) => text(b.querySelector('b')))).toEqual(['Copy the task']);
    click($('.btray [data-icopy]'));
    await settle();
    expect(done().classList.contains('warn')).toBe(true);
    expect(text(done())).toBe("!The task couldn't be copied");
  });

  it('looks_at_the_file_again_while_shown', async () => {
    page = await mount();
    await openPage();
    const asked = core.instructionsGet.mock.calls.length;
    await vi.advanceTimersByTimeAsync(3_100);
    expect(core.instructionsGet.mock.calls.length).toBeGreaterThanOrEqual(asked + 2);
    click($('.btray [data-tback]'));
    await settle();
    const after = core.instructionsGet.mock.calls.length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(core.instructionsGet.mock.calls.length).toBe(after);
  });

  it('broken_file_shows_amber_line', async () => {
    core.instructionsGet.mockResolvedValue(instructions({ broken: true }));
    page = await mount();
    await openPage();
    expect(line().classList.contains('warn')).toBe(true);
    expect(text(line())).toBe("!The file is empty or can't be read: the standard ones work");
    expect(line().querySelector('.bang')?.textContent).toBe('!');
    expect(box().value).toBe(STANDARD_INSTRUCTIONS);
    expect(text($('.btray [data-ireset]'))).toBe('Restore standard');
  });

  it('reset_restores_standard', async () => {
    core.instructionsSet.mockResolvedValue(instructions({ text: 'Keep it short.', edited: true }));
    core.instructionsGet.mockResolvedValue(instructions({ text: 'Keep it short.', edited: true }));
    page = await mount();
    await openPage();
    expect(title()).toBe('Changed by you');
    click($('.btray [data-ireset]'));
    await settle();
    expect(core.instructionsReset).toHaveBeenCalledTimes(1);
    expect(box().value).toBe(STANDARD_INSTRUCTIONS);
    expect(title()).toBe("CUE's standard");
    expect($('.btray [data-ireset]')).toBeNull();
  });

  it('a_failed_save_says_so_in_amber', async () => {
    core.instructionsSet.mockRejectedValue('failed');
    page = await mount();
    await openPage();
    box().value = 'Keep it short.';
    box().dispatchEvent(new Event('input', { bubbles: true }));
    box().dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(done().classList.contains('warn')).toBe(true);
    expect(text(done())).toBe("!The file couldn't be saved");
  });
});
