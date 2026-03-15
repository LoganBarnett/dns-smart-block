let sortDirections = {};

function filterTable() {
  const domainTerm = document.getElementById('domainSearch').value.toLowerCase();
  const reasoningTerm = document.getElementById('reasoningSearch').value.toLowerCase();
  const tbody = document.querySelector('#classificationsTable tbody');
  const rows = Array.from(tbody.querySelectorAll('tr'));

  let visible = 0;
  rows.forEach(row => {
    // Domain is column 0, reasoning is column 4.
    const domain = row.cells[0].textContent.toLowerCase();
    const reasoning = row.cells[4].textContent.toLowerCase();
    const show =
      domain.includes(domainTerm) && reasoning.includes(reasoningTerm);
    row.style.display = show ? '' : 'none';
    if (show) visible++;
  });

  const countEl = document.getElementById('visibleCount');
  if (domainTerm || reasoningTerm) {
    countEl.textContent = `Showing ${visible} of ${rows.length}`;
  } else {
    countEl.textContent = '';
  }
}

function sortTable(tableId, columnIndex) {
  const table = document.getElementById(tableId);
  const tbody = table.querySelector('tbody');
  const rows = Array.from(tbody.querySelectorAll('tr'));

  const key = tableId + ':' + columnIndex;
  const currentDirection = sortDirections[key] || 'asc';
  const newDirection = currentDirection === 'asc' ? 'desc' : 'asc';
  sortDirections = {};
  sortDirections[key] = newDirection;

  rows.sort((a, b) => {
    let aValue = a.cells[columnIndex].textContent.trim();
    let bValue = b.cells[columnIndex].textContent.trim();

    // Try to parse as date/time.
    const aDate = new Date(aValue);
    const bDate = new Date(bValue);

    if (!isNaN(aDate.getTime()) && !isNaN(bDate.getTime())) {
      return newDirection === 'asc'
        ? aDate.getTime() - bDate.getTime()
        : bDate.getTime() - aDate.getTime();
    }

    // Try to parse as number.
    const aNum = parseFloat(aValue);
    const bNum = parseFloat(bValue);

    if (!isNaN(aNum) && !isNaN(bNum)) {
      return newDirection === 'asc' ? aNum - bNum : bNum - aNum;
    }

    // String comparison.
    if (newDirection === 'asc') {
      return aValue.localeCompare(bValue);
    } else {
      return bValue.localeCompare(aValue);
    }
  });

  rows.forEach(row => tbody.appendChild(row));

  // Update header indicators.
  table.querySelectorAll('th').forEach((th, idx) => {
    th.classList.remove('sorted-asc', 'sorted-desc');
    if (idx === columnIndex) {
      th.classList.add(`sorted-${newDirection}`);
    }
  });
}

async function expireDomain(domain) {
  const button = event.target;
  button.disabled = true;
  button.textContent = 'Expiring...';

  try {
    const response = await fetch(`/expire?domain=${encodeURIComponent(domain)}`, {
      method: 'POST',
    });

    if (response.ok) {
      const result = await response.text();
      alert(`Success: ${result}`);
      window.location.reload();
    } else {
      const error = await response.text();
      alert(`Error: ${error}`);
      button.disabled = false;
      button.textContent = 'Expire';
    }
  } catch (error) {
    alert(`Failed to expire domain: ${error.message}`);
    button.disabled = false;
    button.textContent = 'Expire';
  }
}

async function requeueDomain(domain, classificationType) {
  const button = event.target;
  button.disabled = true;
  button.textContent = 'Requeueing...';

  try {
    const response = await fetch(
      `/requeue?domain=${encodeURIComponent(domain)}&classification_type=${encodeURIComponent(classificationType)}`,
      { method: 'POST' }
    );

    if (response.ok) {
      const result = await response.text();
      alert(`Success: ${result}`);
      window.location.reload();
    } else {
      const error = await response.text();
      alert(`Error: ${error}`);
      button.disabled = false;
      button.textContent = 'Requeue';
    }
  } catch (error) {
    alert(`Failed to requeue domain: ${error.message}`);
    button.disabled = false;
    button.textContent = 'Requeue';
  }
}

async function requeueType(classificationType) {
  const button = event.target;
  button.disabled = true;
  button.textContent = 'Requeueing...';

  try {
    const response = await fetch(
      `/requeue/type?classification_type=${encodeURIComponent(classificationType)}`,
      { method: 'POST' }
    );

    if (response.ok) {
      const result = await response.text();
      alert(`Success: ${result}`);
      window.location.reload();
    } else {
      const error = await response.text();
      alert(`Error: ${error}`);
      button.disabled = false;
      button.textContent = `Requeue ${classificationType} errors`;
    }
  } catch (error) {
    alert(`Failed to requeue: ${error.message}`);
    button.disabled = false;
    button.textContent = `Requeue ${classificationType} errors`;
  }
}

async function requeueAll() {
  const button = event.target;
  button.disabled = true;
  button.textContent = 'Requeueing...';

  try {
    const response = await fetch('/requeue/all', { method: 'POST' });

    if (response.ok) {
      const result = await response.text();
      alert(`Success: ${result}`);
      window.location.reload();
    } else {
      const error = await response.text();
      alert(`Error: ${error}`);
      button.disabled = false;
      button.textContent = 'Requeue all errors';
    }
  } catch (error) {
    alert(`Failed to requeue all: ${error.message}`);
    button.disabled = false;
    button.textContent = 'Requeue all errors';
  }
}
