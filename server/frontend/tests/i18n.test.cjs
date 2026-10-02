const test = require('node:test')
const assert = require('node:assert/strict')
const {
  DEFAULT_LANGUAGE, LANGUAGE_STORAGE_KEY, dictionaries, translate,
  readLanguage, persistLanguage, getLanguage, setLanguage, subscribeLanguage,
  intlLocale, number, dateTime, UiError, errorText, pluginField, localizePlugin,
} = require('../node_modules/.i18n-test/i18n/core.js')
const { ApiError } = require('../node_modules/.i18n-test/api.js')

test('dictionaries have identical semantic keys, nonempty text, and matching placeholders', () => {
  const keys = Object.keys(dictionaries.en).sort()
  assert.deepEqual(keys, Object.keys(dictionaries['zh-CN']).sort())
  for (const key of keys) {
    assert.match(key, /^[a-z][A-Za-z]+\.[A-Za-z][A-Za-z0-9]*$/)
    const english = dictionaries.en[key]
    const chinese = dictionaries['zh-CN'][key]
    assert.ok(english.trim(), key)
    assert.ok(chinese.trim(), key)
    assert.doesNotMatch(english, /\p{Script=Han}/u, key)
    const placeholders = (value) => [...value.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort()
    assert.deepEqual(placeholders(english), placeholders(chinese), key)
  }
})

test('English is the explicit default, including invalid or inaccessible storage', () => {
  assert.equal(DEFAULT_LANGUAGE, 'en')
  for (const stored of [null, '', 'fr', 'zh', 'EN']) {
    assert.equal(readLanguage({ getItem: () => stored }), 'en')
  }
  assert.equal(readLanguage({ getItem() { throw new Error('Storage blocked') } }), 'en')
  // No navigator argument or browser-language lookup influences the default.
  const previousWindow = global.window
  global.window = { navigator: { language: 'zh-CN' }, localStorage: { getItem: () => null } }
  try { assert.equal(readLanguage(), 'en') } finally { global.window = previousWindow }
})

test('valid choices persist and storage exceptions never prevent switching', () => {
  const values = new Map()
  const storage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) }
  for (const locale of ['zh-CN', 'en']) {
    persistLanguage(locale, storage)
    assert.equal(values.get(LANGUAGE_STORAGE_KEY), locale)
    assert.equal(readLanguage(storage), locale)
  }
  assert.doesNotThrow(() => persistLanguage('zh-CN', { setItem() { throw new Error('Storage full') } }))
  const previousWindow = global.window
  global.window = { get localStorage() { throw new Error('Storage blocked') } }
  let changes = 0
  const unsubscribe = subscribeLanguage(() => changes++)
  try {
    setLanguage('zh-CN')
    assert.equal(getLanguage(), 'zh-CN')
    assert.equal(changes, 1)
    setLanguage('unsupported')
    assert.equal(getLanguage(), 'zh-CN')
  } finally {
    unsubscribe()
    setLanguage('en')
    global.window = previousWindow
  }
})

test('interpolation handles numbers, repeated parameters, and missing values safely', () => {
  assert.equal(translate('en', 'plugins.counts', { running: 0, installed: 12 }), 'Running: 0 · Installed: 12')
  assert.equal(translate('zh-CN', 'plugins.counts', { running: 1, installed: 2 }), '1 个运行中 · 2 个已安装')
  assert.equal(translate('en', 'connection.disconnect', { name: '$& <script>' }), 'Disconnect $& <script>')
  assert.equal(translate('en', 'connection.disconnect'), 'Disconnect {name}')
})

test('numbers, dates, document language and metadata follow the selected locale', () => {
  const previousDocument = global.document
  const meta = { setAttribute: (key, value) => { meta[key] = value } }
  global.document = { documentElement: { lang: '' }, title: '', querySelector: () => meta }
  try {
    for (const language of ['en', 'zh-CN']) {
      setLanguage(language)
      assert.equal(global.document.documentElement.lang, language)
      assert.equal(global.document.title, translate(language, 'app.title'))
      assert.equal(meta.content, translate(language, 'app.description'))
      const locale = language === 'en' ? 'en-US' : 'zh-CN'
      assert.equal(intlLocale(language), locale)
      assert.equal(number(12345.678, 2), new Intl.NumberFormat(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(12345.678))
      const options = { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' }
      assert.equal(dateTime(0, options), new Intl.DateTimeFormat(locale, options).format(0))
    }
  } finally {
    setLanguage('en')
    global.document = previousDocument
  }
})

function pluginFixture() {
  const field = { key: 'mode', label: 'Mode', description: 'Choose mode', kind: 'select', required: false, default: 'auto', options: ['auto', 'manual'], min: null, max: null }
  return {
    id: 'example', name: 'Example plugin', version: '1', state: 'installed', allowed_masks: ['core0'], default_mask: 'core0',
    max_concurrency: 1, queue_size: 4, request_timeout_ms: 1000,
    configuration: [field], configuration_values: { mode: 'auto' },
    capabilities: [{
      id: 'audio.transcribe', name: 'Transcribe', description: 'Transcribe audio', input_kind: 'audio', output_kind: 'json',
      accepted_content_types: ['audio/wav'], max_input_bytes: 1000, output_content_type: 'application/json', parameters: [field],
      presentation: { renderer: 'speech_transcription', input_hint: 'Choose audio', input_placeholder: 'Audio content', text_examples: ['Do not translate input'] },
    }],
    translations: {
      en: { 'plugin.name': 'Must not override canonical English' },
      'zh-CN': {
        'plugin.name': '示例插件',
        'capabilities.audio.transcribe.name': '语音转写',
        'capabilities.audio.transcribe.presentation.input_hint': '选择音频',
        'capabilities.audio.transcribe.parameters.mode.label': '请求模式',
        'configuration.mode.label': '模式',
        'configuration.mode.options.auto': '自动',
      },
    },
  }
}

test('plugin display fields use dotted keys with canonical English and missing-key fallback', () => {
  const original = pluginFixture()
  const untouched = structuredClone(original)
  const chinese = localizePlugin(original, 'zh-CN')
  assert.equal(chinese.name, '示例插件')
  assert.equal(chinese.capabilities[0].name, '语音转写')
  assert.equal(chinese.capabilities[0].description, 'Transcribe audio')
  assert.equal(chinese.capabilities[0].presentation.input_hint, '选择音频')
  assert.equal(chinese.capabilities[0].presentation.input_placeholder, 'Audio content')
  assert.equal(chinese.capabilities[0].parameters[0].label, '请求模式')
  assert.equal(chinese.configuration[0].label, '模式')
  assert.equal(chinese.configuration[0].optionLabels.auto, '自动')
  assert.deepEqual(chinese.configuration[0].options, original.configuration[0].options)
  assert.deepEqual(chinese.configuration_values, original.configuration_values)
  assert.equal(chinese.capabilities[0].id, original.capabilities[0].id)
  assert.deepEqual(chinese.capabilities[0].presentation.text_examples, original.capabilities[0].presentation.text_examples)
  assert.deepEqual(original, untouched)
  assert.equal(localizePlugin(original, 'en').name, 'Example plugin')
  assert.equal(pluginField({}, 'plugin.name', 'Third-party plugin', 'zh-CN'), 'Third-party plugin')
  assert.equal(localizePlugin({ ...original, translations: undefined }, 'zh-CN').name, 'Example plugin')
})

test('frontend-owned errors switch language while server error text stays canonical', () => {
  const frontend = new UiError('error.managementKey')
  const server = new ApiError('Plugin example is not ready', 409, 'request-1', 'plugin_not_ready')
  const missingBody = new ApiError(null, 503, 'request-2')
  try {
    setLanguage('zh-CN')
    assert.equal(errorText(frontend), translate('zh-CN', 'error.managementKey'))
    assert.equal(frontend.message, translate('en', 'error.managementKey'))
    assert.equal(errorText(server), 'Plugin example is not ready')
    assert.equal(server.status, 409)
    assert.equal(server.requestId, 'request-1')
    assert.equal(errorText(missingBody), '请求失败：503')
    assert.equal(missingBody.message, 'Request failed: 503')
    assert.equal(errorText(new Error('Native browser message')), 'Native browser message')
    setLanguage('en')
    assert.equal(errorText(frontend), translate('en', 'error.managementKey'))
  } finally { setLanguage('en') }
})
