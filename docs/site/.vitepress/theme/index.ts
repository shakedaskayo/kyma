import DefaultTheme from 'vitepress/theme'
import './custom.css'

import Diagram from './components/Diagram.vue'
import Landing from './components/Landing.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Diagram', Diagram)
    app.component('Landing', Landing)
  },
}
