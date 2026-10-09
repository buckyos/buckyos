Wakeup at {{ input.time }}
{% for ev in input.events %}
{{ ev | render_format: "event.summary_text" }}
{% endfor %}
{% for msg in input.messages %}
{{ msg | render_format: "message.xml" }}
{% endfor %}
