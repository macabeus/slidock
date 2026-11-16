// @slidock/player/presenter: the presenter view, a build of its own beside the player's. A host passes `presenter`
// to the player (`PlayerOptions.presenter`, `mount`); a deck page that ships a presenter view embeds the IIFE build
// (`dist/presenter.iife.js`, global `SlidockPresenter`) after the player's and passes it to `Slidock.mount`.
export type { JoinOptions, PresenterOptions, PresenterSupport, PresenterView } from '../player';
export type { PresenterStatus, PresenterWindow } from './window';
export type { NextClick } from './view';
export type { ThemeChoice } from './theme';
export { PRESENTER_CSS } from './style';
export { NEXT_THEME, THEME_LABEL, THEME_STORAGE_KEY } from './theme';
export { createPresenterWindow } from './window';
export { formatDuration, nextClick } from './view';
export { mountPresenter, presenter, presenterSession } from './support';
