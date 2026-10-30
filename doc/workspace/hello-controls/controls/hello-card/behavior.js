// P0 host API (aiws.version === 2): read bindings and config, write through the host.
var cfg = aiws.context.config || {}
var card = document.querySelector('.card')
var done = document.getElementById('done')
done.textContent = cfg.done_label || 'Done'

function render(props) {
  document.getElementById('title').textContent = props.title || '(untitled)'
  document.getElementById('status').textContent = props.status || ''
  document.getElementById('owner').textContent = cfg.show_owner === false ? '' : (props.owner ? '· ' + props.owner : '')
  card.dataset.status = props.status || ''
  done.disabled = props.status === 'done'
}

if (aiws.bindings.task) {
  aiws.input('task').props().then(render)
  // Re-render when the bound record changes (another client, an Agent, a wish).
  aiws.watch('task', function () { aiws.input('task').props().then(render) })
}

done.addEventListener('click', function () {
  // One click = one host-mediated write, with the clicker's identity and rights. A rebuild of this
  // control never replays it (§6.7 "重载不等于再次执行业务").
  aiws.record('task').set({ status: 'done' }).then(function () { aiws.notify('Marked done') })
  // P2 (proposal, not in host API 2): also raise the public event declared in component.json
  // so an instance may bind it to a wish or a command:  aiws.emit('completed', { title: card.dataset.title })
})

aiws.ready()
